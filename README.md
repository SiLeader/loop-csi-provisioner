# loop-csi-provisioner

[日本語](README_ja.md)

A Linux CSI driver that provides filesystem volumes backed by image files in a local directory or an NFS export.

> [!WARNING]
> This project is under development. The Kubernetes manifests are tested on kind, but not yet on a production cluster.

## How it works

1. The **controller** creates a sparse image file in the backing storage and resizes it on expansion.
2. The **node** attaches the image file to a loop device, formats it as ext4 if it is new, and mounts it at the CSI
   staging and publish paths.

## Features and limitations

**Supported**

- ext4 filesystem volumes
- Access modes `SINGLE_NODE_WRITER` and `SINGLE_NODE_READER_ONLY`
- Create, publish, stage, expand (online), unpublish, unstage, and delete
- Multiple controller replicas (active/standby) on shared storage such as NFS
- ext4 multi-mount protection, which prevents two hosts from mounting the same volume

**Not supported**

- Block volumes, other filesystems, mount flags, and volume mount groups
- Snapshots, listing, health, and other optional CSI RPCs (they return `UNIMPLEMENTED`)
- NFS mount options such as `nfsvers` (URLs with query strings are rejected)

## Requirements

- **Build:** Rust/Cargo and `protoc`
- **Node service:** Linux with loop devices and the privileges needed to mount filesystems
- **NFS backing storage:** `mount.nfs` (`nfs-common`, installed by the provided Dockerfile) and host support for NFS
  mounts

## Quick start on Kubernetes

### Helm

The Helm chart is in [charts/loop-csi-provisioner](charts/loop-csi-provisioner) and is published to
`oci://ghcr.io/sileader/charts/loop-csi-provisioner`.

```sh
helm install loop-csi-provisioner oci://ghcr.io/sileader/charts/loop-csi-provisioner \
  --namespace loop-csi --create-namespace \
  --set 'allowedUrlPrefixes={nfs://nfs.example.com/export}' \
  --set 'storageClasses[0].name=nfs-loop' \
  --set 'storageClasses[0].url=nfs://nfs.example.com/export/loop-csi'
```

| Value                                                                     | Description                                                   |
|---------------------------------------------------------------------------|---------------------------------------------------------------|
| `allowedUrlPrefixes`                                                      | **Required.** Storage URLs the driver may mount               |
| `storageClasses`                                                          | StorageClasses to create (optional)                           |
| `controller.replicas`                                                     | Number of controller replicas (see [High availability](#high-availability)) |
| `controller.extraVolumes`, `controller.extraVolumeMounts`                 | Extra volumes for the controller pod, e.g. for `file://` storage |
| `node.extraVolumes`, `node.extraVolumeMounts`                             | Extra volumes for the node pod, e.g. for `file://` storage    |
| `kubeletDir`                                                              | kubelet root, if it is not `/var/lib/kubelet`                 |

For `file://` storage, mount the directory into **both** the controller and node pods. See
[values.yaml](charts/loop-csi-provisioner/values.yaml) for all options.

### Plain manifests

[deploy/manifests](deploy/manifests) contains a CSIDriver, RBAC, a controller Deployment, a node DaemonSet, and
[an example StorageClass](deploy/manifests/storageclass.yaml).

```sh
kubectl apply -k deploy/manifests
```

Then edit and apply the StorageClass. It must set the `url` parameter and `fsType: ext4`. Before use, review the image
tag, the `--allowed-url-prefix` value, and the privileges.

## Backing storage

The StorageClass `url` parameter selects the backing directory:

| URL                                     | Backing storage                          |
|-----------------------------------------|------------------------------------------|
| `file:///srv/loop-csi`                  | A local directory visible to the process |
| `nfs://nfs.example.com/export/loop-csi` | An NFS export mounted by the process     |

The directory has this layout:

```text
<backing directory>/
├── volumes/
│   └── <volume-name>.img       # image file
└── metadata/
    ├── <volume-name>.json      # attachment metadata
    └── lock-<nn>.lock.d/       # controller locks
```

Before using it:

- Create the backing directory, `volumes/`, and `metadata/`, and make them writable by the driver.
- Make the storage reachable from every host that runs a controller or node service using it. A local directory must be
  available at the same path to each of them.

Notes:

- **Image files are sparse.** Space is not reserved up front, so the storage can run out while a volume is in use.
- **Sizes are rounded up** to whole 4 KiB blocks.
- **The URL is part of every volume ID.** Volumes stay tied to the URL they were created with; if an NFS server moves to
  another address, existing PersistentVolumes cannot follow it.
- NFS exports are mounted with the default options of `mount.nfs`.

## Command-line usage

Build:

```sh
cargo build --release
```

Run:

```sh
./target/release/loop-csi-provisioner \
  --listen unix:///csi/csi.sock \
  --base-directory /var/lib/loop-csi-provisioner
```

| Option                                              | Default                          | Description |
|-----------------------------------------------------|----------------------------------|-------------|
| `--listen`                                          | `unix:///csi/csi.sock`           | gRPC address. `tcp://127.0.0.1:1234` is also accepted |
| `--base-directory`                                  | `/var/lib/loop-csi-provisioner`  | Where backing storage is mounted |
| `--identity-api`, `--controller-api`, `--node-api`  | (all APIs)                       | Serve only the selected APIs. Without any of them, all three are served |
| `--node-id` / `NODE_ID`                             |                                  | **Required for the Node API.** Must match the node name the CO uses (in Kubernetes, the node name) |
| `--allowed-url-prefix` (repeatable)                 | (any URL)                        | Allowed storage URLs, matched on `/` boundaries, e.g. `nfs://nfs.example.com/export` |
| `--default-size`                                    | `1073741824` (1 GiB)             | Volume size in bytes when none is requested |
| `--plaintext-log`                                   | JSON logs                        | Print text logs instead of JSON |
| `RUST_LOG`                                          | `info`                           | Log level |

Run `--help` for the full list.

> [!IMPORTANT]
> **Security**
>
> - **Always set `--allowed-url-prefix` in production.** Without it, anyone who can create a PersistentVolume or
>   StorageClass can make the driver mount any URL.
> - The gRPC API has no authentication or TLS. Prefer a Unix socket over `tcp://`.

## Operations

### Multi-mount protection

New volumes are formatted with ext4 multi-mount protection (`mmp`). While one host has a volume mounted, the kernel
refuses to mount it on another host. This protects against cases such as Kubernetes force-detaching a volume from a node
that stopped responding but is still running.

- A cleanly unmounted volume mounts immediately.
- Otherwise (in use elsewhere, or its node crashed), mounting first waits a few MMP intervals, typically tens of seconds.
- Volumes formatted by earlier versions are not protected until you run `tune2fs -O mmp` on the unmounted image.

The node formats only a device whose first MiB is entirely zero, and refuses anything else that is not a recognized ext4
filesystem.

### Controller locking

Controllers serialize operations, including across replicas, by atomically creating a lock directory in the backing
storage.

- Lock ownership never expires, even during an NFS network partition. A controller that lost its NFS lease therefore
  cannot resume alongside a new owner.
- Waiting for the local mutex, mounting, and acquiring the storage lock share a **30-second deadline**. When it
  expires, the request returns `ABORTED` and the CO retries.
- At most 64 lock syscalls and cleanups run at once per process, including syscalls still blocked after their caller
  timed out.

### Recovering a stale lock

If a controller crashes, its operation is interrupted, or lock removal fails, the lock directory remains. Operations in
that lock bucket then keep returning `ABORTED`. Recovery is deliberately manual:

1. Stop and fence every controller that could own or be acquiring the lock.
2. Make sure its pending NFS operations cannot resume later.
3. Remove the empty lock directory with `rmdir` on the backing storage.
4. Restart the controllers.

> [!CAUTION]
> - Deleting the pod on an unreachable node is **not** fencing.
> - Do not remove locks based on their age or a failed health check.

### High availability

The controller Deployment runs one replica by default. To add standby controllers, raise `replicas` in
[controller.yaml](deploy/manifests/controller.yaml) (or `controller.replicas` in the Helm chart).

- Every controller pod must reach the same storage, such as NFS.
- Failover after an in-flight operation crashes requires [stale lock recovery](#recovering-a-stale-lock).

### Upgrading

Both the manifests and the Helm chart use the `Recreate` update strategy: all old controller pods stop before the new
version starts, even with several replicas.

- Controller operations are unavailable during the update. Already mounted volumes remain usable.
- Keep this strategy when upgrading from an earlier locking implementation. Controllers that used only in-process locks
  or file locks cannot coordinate with directory locks and must be stopped first.
- If an old controller's node is unreachable, fence it and its pending storage operations before starting the new
  version.

## Testing

### Unit tests

```sh
cargo test
```

These cover the controller's local file lifecycle and need no privileges.

### End-to-end tests

```sh
test/e2e/run.sh
```

[test/e2e/run.sh](test/e2e/run.sh) builds the image, deploys the driver to a [kind](https://kind.sigs.k8s.io/) cluster,
and takes a PVC through provisioning, writing, online expansion, a read-only remount, multi-mount protection, and
deletion.

| Variable         | Effect                                                                                   |
|------------------|------------------------------------------------------------------------------------------|
| `DEPLOY=helm`    | Install the Helm chart instead of the manifests                                          |
| `BACKEND=nfs`    | Store volumes on an NFS export from an in-cluster NFS server pod instead of a `file://` directory on the kind node |
| `KEEP_CLUSTER=1` | Keep the cluster for debugging                                                           |

Requirements: Docker, kind, kubectl, and a host kernel with loop devices. `BACKEND=nfs` also needs the `nfs` and `nfsd`
kernel modules.

## License

Apache License 2.0. See [LICENSE](LICENSE).
