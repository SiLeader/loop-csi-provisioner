#!/usr/bin/env bash
# End-to-end test on a kind cluster.
#
# Builds the driver image, deploys it with a file:// storage directory on the kind node,
# and takes a PVC through provisioning, writing, online expansion, a read-only remount,
# and deletion. Needs docker, kind, and kubectl; the host kernel must provide loop devices.
#
# Environment:
#   CLUSTER       kind cluster name (default: loop-csi-e2e); an existing one is reused
#   KEEP_CLUSTER  set to 1 to keep the cluster afterwards for debugging
set -euo pipefail

CLUSTER=${CLUSTER:-loop-csi-e2e}
KEEP_CLUSTER=${KEEP_CLUSTER:-0}
IMAGE=loop-csi-provisioner:e2e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
E2E=$ROOT/test/e2e
NODE=$CLUSTER-control-plane
STORAGE=/var/lib/loop-csi-storage
KUBECTL=(kubectl --context "kind-$CLUSTER")

log() { printf '\n=== %s\n' "$*"; }
k() { "${KUBECTL[@]}" "$@"; }

# wait_for DESCRIPTION TIMEOUT_SECONDS COMMAND...: retries COMMAND until it succeeds.
wait_for() {
    local what=$1 timeout=$2
    shift 2
    local deadline=$((SECONDS + timeout))
    until "$@" >/dev/null 2>&1; do
        if ((SECONDS >= deadline)); then
            echo "timed out after ${timeout}s waiting for $what" >&2
            return 1
        fi
        sleep 2
    done
}

dump_diagnostics() {
    log "diagnostics"
    k get pods,pvc,pv,volumeattachments -A -o wide || true
    k get events -A --sort-by=.lastTimestamp | tail -40 || true
    for container in driver csi-provisioner csi-attacher csi-resizer; do
        echo "--- controller/$container"
        k -n loop-csi logs deploy/loop-csi-controller -c "$container" --tail=80 || true
    done
    echo "--- node/driver"
    k -n loop-csi logs ds/loop-csi-node -c driver --tail=200 || true
}

finish() {
    local status=$?
    if ((status != 0)); then
        dump_diagnostics
        echo "E2E FAILED" >&2
    else
        echo "E2E PASSED"
    fi
    if [[ $KEEP_CLUSTER != 1 ]]; then
        kind delete cluster --name "$CLUSTER" >/dev/null 2>&1 || true
    fi
    exit "$status"
}
trap finish EXIT

in_pod() { k exec "$1" -- sh -c "$2"; }
fs_size_kib() { in_pod "$1" "df -k /data | awk 'NR == 2 { print \$2 }'"; }
pvc_capacity() { k get pvc data -o jsonpath='{.status.capacity.storage}'; }

log "building $IMAGE"
docker build -t "$IMAGE" "$ROOT"

log "preparing kind cluster $CLUSTER"
if ! kind get clusters | grep -qx "$CLUSTER"; then
    kind create cluster --name "$CLUSTER" --wait 120s
fi
kind load docker-image "$IMAGE" --name "$CLUSTER"
docker exec "$NODE" mkdir -p "$STORAGE/volumes" "$STORAGE/metadata"

log "deploying the driver"
k apply -k "$E2E"
k -n loop-csi rollout status deploy/loop-csi-controller --timeout=180s
k -n loop-csi rollout status ds/loop-csi-node --timeout=180s

log "a StorageClass outside --allowed-url-prefix is refused"
k apply -f "$E2E/forbidden.yaml"
forbidden_event() {
    k get events --field-selector involvedObject.name=forbidden,reason=ProvisioningFailed \
        -o jsonpath='{.items[*].message}' | grep -q 'not allowed'
}
wait_for "ProvisioningFailed for the forbidden PVC" 120 forbidden_event
[[ $(k get pvc forbidden -o jsonpath='{.status.phase}') == Pending ]]
k delete -f "$E2E/forbidden.yaml" --wait=false

log "provisioning, staging, and writing"
k apply -f "$E2E/workload.yaml"
k wait pod/writer --for=condition=Ready --timeout=300s
pv=$(k get pvc data -o jsonpath='{.spec.volumeName}')
image=$STORAGE/volumes/$pv.img
docker exec "$NODE" test -f "$image"
in_pod writer 'dd if=/dev/urandom of=/data/blob bs=1M count=8 2>/dev/null && sync'
checksum=$(in_pod writer 'sha256sum /data/blob' | cut -d' ' -f1)
size_before=$(fs_size_kib writer)
echo "pv=$pv filesystem=${size_before}KiB checksum=$checksum"
k -n loop-csi exec ds/loop-csi-node -c driver -- dumpe2fs -h "$image" 2>/dev/null |
    grep -q '^Filesystem features:.*\bmmp\b'
# ControllerPublishVolume recorded the attachment next to the image.
docker exec "$NODE" grep -q "\"attachedNode\":\"$NODE\"" "$STORAGE/metadata/$pv.json"

log "multi-mount protection refuses a second writer of the image"
# Stands in for another node that attaches the image while it is in use, e.g. after a
# force-detach: the kernel must refuse the mount instead of corrupting the filesystem.
second_mount_output=$(k -n loop-csi exec -i ds/loop-csi-node -c driver -- bash -s "$image" 2>&1 <<'SCRIPT' || true
set -eu
# `losetup -f` has the kernel create a free device, whose node this /dev may lack; it then
# prints e.g. "/dev/loop17 (lost)".
name=$(losetup -f 2>/dev/null)
name=${name%% *}
name=${name##*/}
IFS=: read -r major minor <"/sys/block/$name/dev"
[[ -e /dev/$name ]] || mknod "/dev/$name" b "$major" "$minor"
losetup "/dev/$name" "$1"
mkdir -p /tmp/second
if mount -t ext4 "/dev/$name" /tmp/second; then
    echo "SECOND MOUNT SUCCEEDED"
    umount /tmp/second
fi
losetup -d "/dev/$name"
SCRIPT
)
echo "$second_mount_output"
if [[ $second_mount_output == *"SECOND MOUNT SUCCEEDED"* ]]; then
    echo "the image was mounted twice" >&2
    exit 1
fi
[[ $second_mount_output == *"already mounted or mount point busy"* ]]

log "expanding the mounted volume"
k patch pvc data --type=merge -p '{"spec":{"resources":{"requests":{"storage":"128Mi"}}}}'
grown() { (($(fs_size_kib writer) > 100 * 1024)); }
wait_for "the filesystem to grow" 300 grown
wait_for "the PVC capacity to be updated" 120 test "$(pvc_capacity)" = 128Mi
echo "filesystem ${size_before}KiB -> $(fs_size_kib writer)KiB"

log "remounting read-only in a new pod"
k delete pod writer --wait=true
k apply -f "$E2E/reader.yaml"
k wait pod/reader --for=condition=Ready --timeout=300s
[[ $(in_pod reader 'sha256sum /data/blob' | cut -d' ' -f1) == "$checksum" ]]
if in_pod reader 'touch /data/should-fail' 2>/dev/null; then
    echo "a read-only mount accepted a write" >&2
    exit 1
fi
(($(fs_size_kib reader) > 100 * 1024))

log "deleting the volume"
k delete pod reader --wait=true
k delete pvc data --wait=true
pv_gone() { ! k get pv "$pv"; }
wait_for "the PV to be deleted" 180 pv_gone
docker exec "$NODE" test ! -e "$image"
docker exec "$NODE" test ! -e "$STORAGE/metadata/$pv.json"
# Unstaging must have detached the loop device.
if docker exec "$NODE" sh -c "cat /sys/block/loop*/loop/backing_file 2>/dev/null" | grep -q "$pv"; then
    echo "a loop device is still attached to $image" >&2
    exit 1
fi
