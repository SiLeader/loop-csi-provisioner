# loop-csi-provisioner

[日本語](README_ja.md)

`loop-csi-provisioner` is a Linux CSI driver under development. It is designed to provide filesystem volumes backed by
image files in a local directory or an NFS export. The controller creates and resizes an image file; the node attaches
that file to a loop device, formats it as ext4 when needed, and mounts it at the CSI staging and publish paths.

## Storage layout

The StorageClass `url` parameter selects the backing directory:

| URL                                     | Backing storage                          |
|-----------------------------------------|------------------------------------------|
| `file:///srv/loop-csi`                  | A local directory visible to the process |
| `nfs://nfs.example.com/export/loop-csi` | An NFS export mounted by the process     |

The driver uses `volumes/<volume-name>.img` for image files and `metadata/<volume-name>.json` for attachment metadata
under that directory. The backing directory and both subdirectories must already exist and be writable by the driver.
For NFS, the export must be reachable from every host running a controller or node service that uses it. A local
directory must likewise be available at the same path to each service that needs the volume.

Image files are sparse, so the backing storage is not reserved up front and can run out of space while a volume is in
use. The controller keeps a per-volume lock but does not coordinate several controller processes that share one backing
directory; run a single active controller.

New volumes are formatted with ext4 multi-mount protection (`mmp`): while one host has a volume mounted, the kernel
refuses to mount it on another, e.g. after Kubernetes force-detaches a volume from a node that stopped responding but is
still running. A cleanly unmounted volume mounts immediately; one that was not (in use elsewhere, or its node crashed)
first waits a few MMP intervals, typically tens of seconds. Volumes formatted by earlier versions lack this protection
until `tune2fs -O mmp` is run on the unmounted image. Volume sizes are rounded up to whole 4 KiB blocks.

The storage URL is part of every volume ID, so volumes stay tied to the URL they were created with: if an NFS server
moves to another address, existing PersistentVolumes cannot follow it. NFS exports are mounted with the default options
of `mount.nfs`; URLs with query strings are rejected, so options such as `nfsvers` cannot be set yet.

The driver supports ext4 filesystem volumes with `SINGLE_NODE_WRITER` or `SINGLE_NODE_READER_ONLY` access. It does not
support block volumes, other filesystems, mount flags, or volume mount groups. The node runs `mkfs.ext4` when formatting
a new volume and `resize2fs` when expanding one; it formats only a device whose first MiB is entirely zero and refuses
anything else that is not a recognized ext4 filesystem. Loop-device and local mount operations use Linux system calls,
and NFS exports are mounted with the `mount` command, so `mount.nfs` (`nfs-common`) must be installed (the provided
Dockerfile does this). Run the node
service on Linux with access to loop devices and the permissions needed to mount filesystems; NFS backing storage also
requires the host to support NFS mounts.

## Build and run

Build with Rust/Cargo and `protoc` installed:

```sh
cargo build --release
```

Start the CSI gRPC server, for example:

```sh
./target/release/loop-csi-provisioner \
  --listen unix:///csi/csi.sock \
  --base-directory /var/lib/loop-csi-provisioner
```

With no API selection flags, the process serves the Identity, Controller, and Node APIs. Pass one or more of
`--identity-api`, `--controller-api`, and `--node-api` to serve only those selected APIs. `--listen` also accepts
`tcp://127.0.0.1:1234`. `--default-size` sets the fallback volume size in bytes (default: 1 GiB). `--plaintext-log`
selects text logs instead of JSON logs; the log level comes from `RUST_LOG` (default `info`).
`--allowed-url-prefix` (repeatable) restricts which storage URLs may be mounted, matching on `/` boundaries, e.g.
`--allowed-url-prefix nfs://nfs.example.com/export`. Because the URL is part of every volume ID, set it in production:
without it, whoever can create a PersistentVolume or StorageClass can make the driver mount any URL. The gRPC API has no
authentication or TLS, so prefer a Unix socket over `tcp://`. The Node API requires `--node-id` (or `NODE_ID`), which must match the name the
CO uses for the node (in Kubernetes, the node name). See `--help` for all options.

## Kubernetes status

[deploy/manifests](deploy/manifests) contains a CSIDriver, RBAC, a controller Deployment, a node DaemonSet, and
[an example StorageClass](deploy/manifests/storageclass.yaml) that sets the required `url` parameter and `fsType: ext4`.
Apply the driver with `kubectl apply -k deploy/manifests`, then adapt and apply the StorageClass. The manifests are
exercised on kind (see below) but not yet on a production cluster; review the image tag, the `--allowed-url-prefix`
value, and the privileges before use.

The basic create, publish, stage, expand, unpublish, unstage, and delete operations are implemented. Snapshot, listing,
health, and other optional CSI RPCs return `UNIMPLEMENTED`. Node mounting requires a privileged Linux host with loop
devices.

Unit tests (`cargo test`) cover the controller's local file lifecycle and need no privileges. The end-to-end test
[test/e2e/run.sh](test/e2e/run.sh) builds the image, deploys the manifests to a [kind](https://kind.sigs.k8s.io/)
cluster with a `file://` storage directory on the kind node, and takes a PVC through provisioning, writing, online
expansion, a read-only remount, multi-mount protection, and deletion. It needs Docker, kind, kubectl, and a host kernel
with loop devices; set `KEEP_CLUSTER=1` to keep the cluster for debugging. NFS backing storage is not covered yet.

## License

Apache License 2.0. See [LICENSE](LICENSE).
