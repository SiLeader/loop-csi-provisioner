# loop-csi-provisioner

[English](README.md)

`loop-csi-provisioner` は開発途中の Linux 向け CSI ドライバーです。ローカルディレクトリまたは NFS
エクスポート内に作成したイメージファイルを使い、ファイルシステムボリュームを提供することを目指しています。Controller
がイメージファイルを作成・拡張し、Node がそのファイルをループデバイスに接続して、必要に応じて ext4 で初期化した後、CSI
のステージング先と公開先にマウントします。

## ストレージの配置

StorageClass の `url` パラメーターで保存先のディレクトリを指定します。

| URL                                     | 保存先                                  |
|-----------------------------------------|-----------------------------------------|
| `file:///srv/loop-csi`                  | プロセスから見えるローカルディレクトリ  |
| `nfs://nfs.example.com/export/loop-csi` | プロセスがマウントする NFS エクスポート |

指定したディレクトリの下に、イメージファイルを `volumes/<ボリューム名>.img`、接続情報を `metadata/<ボリューム名>.json`
として保存します。保存先と二つのサブディレクトリは、事前に作成し、ドライバーが書き込めるようにしてください。NFS
を使う場合は、そのボリュームを扱う Controller と Node
の各ホストからエクスポートに接続できる必要があります。ローカルディレクトリも、ボリュームを扱う各サービスから同じパスで参照できる必要があります。

イメージファイルはスパースファイルなので、保存先の容量は事前に確保されず、ボリュームの使用中に容量不足になることがあります。Controller
はボリューム単位のロックを持ちますが、同じ保存先を共有する複数の Controller プロセス間の調停はしないため、有効な Controller
は一つだけにしてください。

新しいボリュームは ext4 の Multi-Mount Protection（`mmp`）付きで初期化します。あるホストがボリュームをマウントしている間は、
別のホストでのマウントをカーネルが拒否します（応答しなくなったがまだ動いているノードから Kubernetes が強制 detach した場合など）。
正常にアンマウントされたボリュームはすぐにマウントできますが、そうでない場合（ほかで使用中、またはノードがクラッシュした場合）は、
マウント前に MMP の間隔数回分（通常は数十秒）待ちます。以前のバージョンで初期化したボリュームは、アンマウントした状態のイメージに
`tune2fs -O mmp` を実行するまでこの保護を受けません。ボリュームのサイズは 4 KiB 単位に切り上げます。

保存先 URL はボリューム ID に含まれるため、ボリュームは作成時の URL に結び付きます。NFS サーバーのアドレスが変わっても、既存の
PersistentVolume はそれに追従できません。NFS エクスポートは `mount.nfs` の既定のオプションでマウントします。クエリ文字列付きの URL
は拒否するため、`nfsvers` などのオプションはまだ指定できません。

対応するのは ext4 ファイルシステムボリュームと `SINGLE_NODE_WRITER` / `SINGLE_NODE_READER_ONLY` アクセスモードです。
ブロックボリューム、ほかのファイルシステム、マウントフラグ、ボリュームマウントグループには対応していません。Node は新規ボリュームの
初期化時に `mkfs.ext4`、拡張時に `resize2fs` を実行します。初期化するのは先頭 1 MiB がすべて 0 のデバイスだけで、それ以外で ext4
と認識できないものは初期化せずに拒否します。ループデバイスとローカルのマウントには Linux のシステムコールを使い、NFS は `mount`
コマンドでマウントするため、`mount.nfs`（`nfs-common`）が必要です（同梱の Dockerfile でインストールしています）。
Node サービスは、ループデバイスへのアクセスとマウントに必要な権限を備えた Linux 環境で実行してください。NFS を保存先に使う場合は、
ホスト側で NFS マウントも利用できる必要があります。

## ビルドと起動

Rust/Cargo と `protoc` を用意してビルドします。

```sh
cargo build --release
```

CSI gRPC サーバーの起動例です。

```sh
./target/release/loop-csi-provisioner \
  --listen unix:///csi/csi.sock \
  --base-directory /var/lib/loop-csi-provisioner
```

API 選択フラグを省略すると、Identity、Controller、Node の各 API を提供します。`--identity-api`、`--controller-api`、
`--node-api` を一つ以上指定すると、指定した API だけを提供します。`--listen` には `tcp://127.0.0.1:1234` も指定できます。
`--default-size` はサイズ指定のないボリュームの容量をバイト単位で設定します（既定値は 1 GiB）。`--plaintext-log` を指定すると
JSON 形式ではなくテキスト形式でログを出力します。ログレベルは `RUST_LOG` で指定します（既定は `info`）。
`--allowed-url-prefix`（複数指定可）は、マウントを許可する保存先 URL を `/` 区切りの前方一致で制限します。例:
`--allowed-url-prefix nfs://nfs.example.com/export`。URL はすべてのボリューム ID に含まれるため、本番では必ず指定してください。指定しないと、
PersistentVolume や StorageClass を作成できる人が、任意の URL をドライバーにマウントさせられます。gRPC API には認証も TLS もないため、
`tcp://` ではなく Unix ソケットを使ってください。Node API には `--node-id`（または `NODE_ID`）が必要で、CO がそのノードに
使う名前（Kubernetes ではノード名）と一致させてください。全オプションは `--help` で確認できます。

## Kubernetes 対応状況

[deploy/manifests](deploy/manifests) に、CSIDriver、RBAC、Controller の Deployment、Node の DaemonSet、
[StorageClass の例](deploy/manifests/storageclass.yaml)（必須の `url` パラメーターと `fsType: ext4` を指定）を置いています。
ドライバーは `kubectl apply -k deploy/manifests` で適用し、StorageClass は環境に合わせて編集してから適用してください。マニフェストは kind
上では検証していますが（後述）、本番クラスターではまだ試していません。使用前に、イメージのタグ、`--allowed-url-prefix` の値、権限を確認してください。

基本的な作成、公開、ステージング、拡張、公開解除、ステージング解除、削除を実装しました。スナップショット、一覧、ヘルスチェックなどのオプションの
CSI RPC は `UNIMPLEMENTED` を返します。Node でのマウントには、ループデバイスを備えた特権付き Linux
ホストが必要です。

単体テスト（`cargo test`）は Controller のローカルファイルのライフサイクルを対象とし、特権は不要です。E2E テスト
[test/e2e/run.sh](test/e2e/run.sh) はイメージをビルドし、kind ノード上の `file://` 保存先を使って [kind](https://kind.sigs.k8s.io/)
クラスターにマニフェストを適用します。そのうえで PVC の作成、書き込み、オンライン拡張、読み取り専用での再マウント、Multi-Mount Protection、
削除までを確認します。Docker、kind、kubectl と、ループデバイスを使えるホストカーネルが必要です。`KEEP_CLUSTER=1` を指定すると、
デバッグ用にクラスターを残します。NFS の保存先はまだ対象外です。

## ライセンス

Apache License 2.0。[LICENSE](LICENSE) を参照してください。
