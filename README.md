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
authentication or TLS, so prefer a Unix socket over `tcp://`. The Node API uses `NODE_ID` as its node identifier, falling back to
`/etc/hostname`. See `--help` for all options.

## Kubernetes status

[deploy/manifests](deploy/manifests) contains a CSIDriver, RBAC, a controller Deployment, a node DaemonSet, and
[an example StorageClass](deploy/manifests/storageclass.yaml) that sets the required `url` parameter and `fsType: ext4`.
These manifests have not been applied to a real cluster yet; review the image tags, the `--allowed-url-prefix` value,
and the privileges before use.

The basic create, publish, stage, expand, unpublish, unstage, and delete operations are implemented. Snapshot, listing,
health, and other optional CSI RPCs return `UNIMPLEMENTED`. Node mounting requires a privileged Linux host with loop
devices; the automated tests cover the local file lifecycle but do not exercise privileged mount operations or NFS.

## License

Apache License 2.0. See [LICENSE](LICENSE).
