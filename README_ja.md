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

対応するのは ext4 ファイルシステムボリュームと `SINGLE_NODE_WRITER` / `SINGLE_NODE_READER_ONLY` アクセスモードです。
ブロックボリューム、ほかのファイルシステム、マウントフラグ、ボリュームマウントグループには対応していません。Node は新規ボリュームの
初期化時に `mkfs.ext4`、拡張時に `resize2fs` を実行します。ループデバイスとマウントの操作には Linux のシステムコールを使用します。
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
JSON 形式ではなくテキスト形式でログを出力します。Node API は `NODE_ID` をノード識別子として使い、設定されていない場合は
`/etc/hostname` を参照します。全オプションは `--help` で確認できます。

## Kubernetes 対応状況

このリポジトリには、Kubernetes 向けの完全なデプロイ構成はまだありません。[StorageClass の例](deploy/manifests/storageclass.yaml)
には必須の `url` パラメーターと `fsType: ext4` を指定しています。Controller、Node DaemonSet、CSI
サイドカー、ソケット、権限は別途設定する必要があります。

基本的な作成、公開、ステージング、拡張、公開解除、ステージング解除、削除を実装しました。スナップショット、一覧、ヘルスチェックなどのオプションの
CSI RPC は `UNIMPLEMENTED` を返します。Node でのマウントには、ループデバイスを備えた特権付き Linux
ホストが必要です。自動テストはローカルファイルのライフサイクルを対象としており、特権が必要なマウント処理や NFS は対象外です。

## ライセンス

Apache License 2.0。[LICENSE](LICENSE) を参照してください。
