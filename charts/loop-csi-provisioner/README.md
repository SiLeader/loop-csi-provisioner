# loop-csi-provisioner

A Linux CSI driver that provides ext4 volumes backed by image files in a local directory or an NFS export.

> [!WARNING]
> This project is under development. The chart is tested on kind, but not yet on a production cluster.

See the [project README](https://github.com/SiLeader/loop-csi-provisioner#readme) for how the driver works, its
limitations, and operations such as stale lock recovery.

## How it works

1. The **controller** creates a sparse image file in the backing storage and resizes it on expansion.
2. The **node** attaches the image file to a loop device, formats it as ext4 if it is new, and mounts it at the CSI
   staging and publish paths.

Supported: ext4 filesystem volumes, single-node access modes (`ReadWriteOnce`), online expansion, and multiple
controller replicas (active/standby) on shared storage such as NFS. Block volumes and snapshots are not supported.

## Prerequisites

- Kubernetes 1.25+
- Linux nodes with loop devices
- For NFS backing storage, host support for NFS mounts

## Installing

```sh
helm install loop-csi-provisioner oci://ghcr.io/sileader/charts/loop-csi-provisioner \
  --namespace loop-csi --create-namespace \
  --set 'allowedUrlPrefixes={nfs://nfs.example.com/export}' \
  --set 'storageClasses[0].name=nfs-loop' \
  --set 'storageClasses[0].url=nfs://nfs.example.com/export/loop-csi'
```

`allowedUrlPrefixes` is **required**; the chart refuses to render without it. The URL is part of every volume ID, so
without a restriction anyone who can create a PersistentVolume or StorageClass could make the privileged driver mount
any URL.

### Backing storage

The StorageClass `url` selects the backing directory:

| URL                                     | Backing storage                          |
|-----------------------------------------|------------------------------------------|
| `file:///srv/loop-csi`                  | A local directory visible to the pods    |
| `nfs://nfs.example.com/export/loop-csi` | An NFS export mounted by the driver      |

Before use, create the backing directory with `volumes/` and `metadata/` subdirectories writable by the driver.

For `file://` storage, mount the directory at the same path into **both** the controller and node pods:

```yaml
allowedUrlPrefixes:
  - file:///srv/loop-csi
storageClasses:
  - name: local-loop
    url: file:///srv/loop-csi
controller:
  extraVolumes:
    - name: storage
      hostPath:
        path: /srv/loop-csi
        type: Directory
  extraVolumeMounts:
    - name: storage
      mountPath: /srv/loop-csi
node:
  extraVolumes:
    - name: storage
      hostPath:
        path: /srv/loop-csi
        type: Directory
  extraVolumeMounts:
    - name: storage
      mountPath: /srv/loop-csi
```

Image files are sparse: space is not reserved up front, so the storage can run out while a volume is in use.

## Values

| Value                                                     | Default                                 | Description |
|-----------------------------------------------------------|-----------------------------------------|-------------|
| `allowedUrlPrefixes`                                      | `[]`                                    | **Required.** Storage URLs the driver may mount, matched on `/` boundaries |
| `storageClasses`                                          | `[]`                                    | StorageClasses to create (`name`, `url`, `isDefault`, `fsType`, `reclaimPolicy`, `allowVolumeExpansion`, `volumeBindingMode`, `labels`, `annotations`) |
| `defaultSize`                                             | `null` (1 GiB)                          | Volume size in bytes when a PVC requests none |
| `image.repository`                                        | `ghcr.io/sileader/loop-csi-provisioner` | Driver image |
| `image.tag`                                               | `""` (`v<appVersion>`)                  | Driver image tag |
| `log.level`                                               | `info`                                  | `RUST_LOG` filter |
| `log.format`                                              | `json`                                  | `json` or `text` |
| `kubeletDir`                                              | `/var/lib/kubelet`                      | kubelet root directory on the host |
| `csiDriver.create`                                        | `true`                                  | Create the CSIDriver object |
| `serviceAccount.create`, `rbac.create`                    | `true`                                  | Create the controller ServiceAccount and RBAC |
| `controller.replicas`                                     | `1`                                     | Controller replicas (standbys need shared storage) |
| `controller.extraVolumes`, `controller.extraVolumeMounts` | `[]`                                    | Extra volumes for the controller pod |
| `node.extraVolumes`, `node.extraVolumeMounts`             | `[]`                                    | Extra volumes for the node pod |
| `controller.*`, `node.*`                                  |                                         | `extraArgs`, `extraEnv`, `resources`, `nodeSelector`, `tolerations`, `affinity`, etc. |
| `sidecars.*`                                              |                                         | Images and settings of the CSI sidecars |

See [values.yaml](https://github.com/SiLeader/loop-csi-provisioner/blob/master/charts/loop-csi-provisioner/values.yaml)
for all options.

## Upgrading

The controller uses the `Recreate` update strategy: all old controller pods stop before the new version starts.
Controller operations are unavailable during the update, but already mounted volumes remain usable.

## License

Apache License 2.0.
