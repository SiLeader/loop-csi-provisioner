# loop-csi-provisioner

[English](README.md)

`loop-csi-provisioner` は開発途中の Linux 向け CSI ドライバーです。ローカルディレクトリまたは NFS エクスポート内に作成したイメージファイルを使い、ファイルシステムボリュームを提供することを目指しています。Controller がイメージファイルを作成・拡張し、Node がそのファイルをループデバイスに接続して、必要に応じて ext4 で初期化した後、CSI のステージング先と公開先にマウントします。

## ストレージの配置

StorageClass の `url` パラメーターで保存先のディレクトリを指定します。

| URL | 保存先 |
| --- | --- |
| `file:///srv/loop-csi` | プロセスから見えるローカルディレクトリ |
| `nfs://nfs.example.com/export/loop-csi` | プロセスがマウントする NFS エクスポート |

指定したディレクトリの下に、イメージファイルを `volumes/<ボリューム名>.img`、接続情報を `metadata/<ボリューム名>.json` として保存します。保存先と二つのサブディレクトリは、事前に作成し、ドライバーが書き込めるようにしてください。NFS を使う場合は、そのボリュームを扱う Controller と Node の各ホストからエクスポートに接続できる必要があります。ローカルディレクトリも、ボリュームを扱う各サービスから同じパスで参照できる必要があります。

Node の実装は `mountpoint`、`mount`、`losetup`、`blkid`、`mkfs.ext4`、`resize2fs` を使用します。NFS ではマウント用のヘルパーも必要です。これらの操作に必要な権限とループデバイスを備えた Linux 環境で実行してください。

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

API 選択フラグを省略すると、Identity、Controller、Node の各 API を提供します。`--identity-api`、`--controller-api`、`--node-api` を一つ以上指定すると、指定した API だけを提供します。`--listen` には `tcp://127.0.0.1:1234` も指定できます。`--default-size` はサイズ指定のないボリュームの容量をバイト単位で設定します（既定値は 1 GiB）。`--plaintext-log` を指定すると JSON 形式ではなくテキスト形式でログを出力します。全オプションは `--help` で確認できます。

## Kubernetes 対応状況

このリポジトリには、Kubernetes 向けの完全なデプロイ構成はまだありません。[`manifests/storageclass.yaml`](manifests/storageclass.yaml) は `url` パラメーターの指定例であり、**そのまま適用できる StorageClass ではありません**。`provisioner` が Identity サービスの返す名前と異なり、`fsType` も Node が使用する ext4 と一致していません。

現時点では、ボリュームの作成から利用までを通して実行できません。特に、`CreateVolume` は URL を含むボリューム ID を要求するパス処理に CSI ボリューム名のみを渡します。また、ローカルの `file://` パスではシンボリックリンクの作成前に同じパスを作成するため衝突します。ライフサイクル処理や機能通知の RPC にも `todo!()` または未実装エラーが残っています。この版を本番データに使用したり、サンプルの StorageClass で PVC を作成できると想定したりしないでください。

## ライセンス

Apache License 2.0。[LICENSE](LICENSE) を参照してください。
