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
`tcp://` ではなく Unix ソケットを使ってください。Node API は `NODE_ID` をノード識別子として使い、設定されていない場合は
`/etc/hostname` を参照します。全オプションは `--help` で確認できます。

## Kubernetes 対応状況

[deploy/manifests](deploy/manifests) に、CSIDriver、RBAC、Controller の Deployment、Node の DaemonSet、
[StorageClass の例](deploy/manifests/storageclass.yaml)（必須の `url` パラメーターと `fsType: ext4` を指定）を置いています。
これらのマニフェストは実際のクラスターではまだ試していません。使用前に、イメージのタグ、`--allowed-url-prefix` の値、権限を確認してください。

基本的な作成、公開、ステージング、拡張、公開解除、ステージング解除、削除を実装しました。スナップショット、一覧、ヘルスチェックなどのオプションの
CSI RPC は `UNIMPLEMENTED` を返します。Node でのマウントには、ループデバイスを備えた特権付き Linux
ホストが必要です。自動テストはローカルファイルのライフサイクルを対象としており、特権が必要なマウント処理や NFS は対象外です。

## ライセンス

Apache License 2.0。[LICENSE](LICENSE) を参照してください。
