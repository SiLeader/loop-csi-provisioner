# loop-csi-provisioner

[English](README.md)

ローカルディレクトリまたは NFS エクスポート上のイメージファイルを使って、ファイルシステムボリュームを提供する Linux 向け CSI ドライバーです。

> [!WARNING]
> 開発途中のプロジェクトです。Kubernetes マニフェストは kind 上で検証していますが、本番クラスターではまだ試していません。

## 仕組み

1. **Controller** が保存先にスパースなイメージファイルを作成し、拡張時にはサイズを変更します。
2. **Node** がイメージファイルをループデバイスに接続し、新規ボリュームであれば ext4 で初期化して、CSI のステージング先と公開先にマウントします。

## 機能と制限

**対応しているもの**

- ext4 ファイルシステムボリューム
- アクセスモード `SINGLE_NODE_WRITER` と `SINGLE_NODE_READER_ONLY`
- 作成、公開、ステージング、拡張（オンライン）、公開解除、ステージング解除、削除
- NFS などの共有ストレージを使った複数レプリカの Controller（アクティブ／スタンバイ）
- 同じボリュームを二つのホストでマウントすることを防ぐ ext4 Multi-Mount Protection

**対応していないもの**

- ブロックボリューム、ext4 以外のファイルシステム、マウントフラグ、ボリュームマウントグループ
- スナップショット、一覧、ヘルスチェックなどのオプションの CSI RPC（`UNIMPLEMENTED` を返します）
- `nfsvers` などの NFS マウントオプション（クエリ文字列付きの URL は拒否します）

## 必要なもの

- **ビルド:** Rust/Cargo と `protoc`
- **Node サービス:** ループデバイスを使え、ファイルシステムのマウントに必要な権限を持つ Linux 環境
- **NFS を保存先に使う場合:** `mount.nfs`（`nfs-common`。同梱の Dockerfile でインストール済み）と、ホスト側の NFS マウント対応

## Kubernetes へのデプロイ

### Helm

Helm チャートは [charts/loop-csi-provisioner](charts/loop-csi-provisioner) にあり、
`oci://ghcr.io/sileader/charts/loop-csi-provisioner` でも公開しています。

```sh
helm install loop-csi-provisioner oci://ghcr.io/sileader/charts/loop-csi-provisioner \
  --namespace loop-csi --create-namespace \
  --set 'allowedUrlPrefixes={nfs://nfs.example.com/export}' \
  --set 'storageClasses[0].name=nfs-loop' \
  --set 'storageClasses[0].url=nfs://nfs.example.com/export/loop-csi'
```

| 値                                                        | 説明                                                               |
|-----------------------------------------------------------|--------------------------------------------------------------------|
| `allowedUrlPrefixes`                                      | **必須。** ドライバーがマウントしてよい保存先 URL                  |
| `storageClasses`                                          | 作成する StorageClass（任意）                                      |
| `controller.replicas`                                     | Controller のレプリカ数（[高可用性](#高可用性)を参照）             |
| `controller.extraVolumes`, `controller.extraVolumeMounts` | Controller Pod に追加するボリューム（`file://` の保存先用など）    |
| `node.extraVolumes`, `node.extraVolumeMounts`             | Node Pod に追加するボリューム（`file://` の保存先用など）          |
| `kubeletDir`                                              | kubelet のルート（`/var/lib/kubelet` でない場合）                  |

`file://` の保存先を使う場合は、そのディレクトリを Controller と Node の**両方**の Pod にマウントしてください。
すべての設定項目は [values.yaml](charts/loop-csi-provisioner/values.yaml) を参照してください。

### マニフェスト

[deploy/manifests](deploy/manifests) に、CSIDriver、RBAC、Controller の Deployment、Node の DaemonSet、
[StorageClass の例](deploy/manifests/storageclass.yaml) があります。

```sh
kubectl apply -k deploy/manifests
```

その後、StorageClass を環境に合わせて編集して適用してください。`url` パラメーターと `fsType: ext4` の指定が必要です。
使用前に、イメージのタグ、`--allowed-url-prefix` の値、権限を確認してください。

## 保存先

StorageClass の `url` パラメーターで保存先のディレクトリを指定します。

| URL                                     | 保存先                                  |
|-----------------------------------------|-----------------------------------------|
| `file:///srv/loop-csi`                  | プロセスから見えるローカルディレクトリ  |
| `nfs://nfs.example.com/export/loop-csi` | プロセスがマウントする NFS エクスポート |

ディレクトリの構成は次のとおりです。

```text
<保存先ディレクトリ>/
├── volumes/
│   └── <ボリューム名>.img       # イメージファイル
└── metadata/
    ├── <ボリューム名>.json      # 接続情報
    └── lock-<nn>.lock.d/        # Controller のロック
```

使用前の準備:

- 保存先ディレクトリ、`volumes/`、`metadata/` を作成し、ドライバーが書き込めるようにしてください。
- 保存先を使う Controller と Node の各ホストから保存先にアクセスできるようにしてください。ローカルディレクトリの場合は、
  各ホストで同じパスから参照できる必要があります。

注意点:

- **イメージファイルはスパースファイルです。** 容量は事前に確保されないため、ボリュームの使用中に保存先が容量不足になることがあります。
- **サイズは 4 KiB 単位に切り上げます。**
- **保存先 URL はボリューム ID に含まれます。** ボリュームは作成時の URL に結び付くため、NFS サーバーのアドレスが変わっても、
  既存の PersistentVolume は追従できません。
- NFS エクスポートは `mount.nfs` の既定のオプションでマウントします。

## コマンドラインでの使い方

ビルド:

```sh
cargo build --release
```

起動:

```sh
./target/release/loop-csi-provisioner \
  --listen unix:///csi/csi.sock \
  --base-directory /var/lib/loop-csi-provisioner
```

| オプション                                          | 既定値                          | 説明 |
|-----------------------------------------------------|---------------------------------|------|
| `--listen`                                          | `unix:///csi/csi.sock`          | gRPC の待ち受けアドレス。`tcp://127.0.0.1:1234` も指定可能 |
| `--base-directory`                                  | `/var/lib/loop-csi-provisioner` | 保存先をマウントするディレクトリ |
| `--identity-api`, `--controller-api`, `--node-api`  | （全 API）                      | 指定した API だけを提供。どれも指定しなければ三つすべてを提供 |
| `--node-id` / `NODE_ID`                             |                                 | **Node API では必須。** CO がそのノードに使う名前（Kubernetes ではノード名）と一致させる |
| `--allowed-url-prefix`（複数指定可）                | （任意の URL）                  | 許可する保存先 URL。`/` 区切りの前方一致。例: `nfs://nfs.example.com/export` |
| `--default-size`                                    | `1073741824`（1 GiB）           | サイズ指定のないボリュームの容量（バイト） |
| `--plaintext-log`                                   | JSON 形式                       | ログを JSON ではなくテキスト形式で出力 |
| `RUST_LOG`                                          | `info`                          | ログレベル |

全オプションは `--help` で確認できます。

> [!IMPORTANT]
> **セキュリティ**
>
> - **本番では必ず `--allowed-url-prefix` を指定してください。** 指定しないと、PersistentVolume や StorageClass
>   を作成できる人が、任意の URL をドライバーにマウントさせられます。
> - gRPC API には認証も TLS もありません。`tcp://` ではなく Unix ソケットを使ってください。

## 運用

### Multi-Mount Protection

新しいボリュームは ext4 の Multi-Mount Protection（`mmp`）付きで初期化します。あるホストがボリュームをマウントしている間は、
別のホストでのマウントをカーネルが拒否します。これにより、応答しなくなったがまだ動いているノードから Kubernetes
がボリュームを強制 detach した場合などでも、二重マウントを防ぎます。

- 正常にアンマウントされたボリュームはすぐにマウントできます。
- そうでない場合（ほかで使用中、またはノードがクラッシュした場合）は、マウント前に MMP の間隔数回分（通常は数十秒）待ちます。
- 以前のバージョンで初期化したボリュームは、アンマウントした状態のイメージに `tune2fs -O mmp` を実行するまで保護されません。

Node が初期化するのは先頭 1 MiB がすべて 0 のデバイスだけです。それ以外で ext4 と認識できないものは、初期化せずに拒否します。

### Controller のロック

Controller は、保存先にロックディレクトリをアトミックに作成することで、複数レプリカ間も含めて操作を直列化します。

- NFS のネットワーク分断中もロックの所有権は失効しません。そのため、NFS のリースを失った Controller が再開して、
  新しい所有者と同時に動作することはありません。
- ローカル mutex の待機、マウント、保存先のロック取得には、合計 **30 秒の期限**があります。期限を超えると `ABORTED` を返し、
  CO が再試行します。
- 同時に実行するロック用システムコールとクリーンアップは、プロセスごとに最大 64 件です。呼び出し元のタイムアウト後も
  ブロックしているシステムコールもこれに含まれます。

### 残ったロックの復旧

Controller がクラッシュした場合、操作が中断された場合、またはロックの削除に失敗した場合は、ロックディレクトリが残ります。
その後、同じロックバケットの操作は `ABORTED` を返し続けます。復旧は意図的に手動にしています。

1. そのロックを所有している可能性がある、または取得中のすべての Controller を停止してフェンシングします。
2. 保留中の NFS 操作が後から再開しないことを確認します。
3. 保存先の空のロックディレクトリを `rmdir` で削除します。
4. Controller を再起動します。

> [!CAUTION]
> - 到達できないノードの Pod を削除するだけでは、フェンシングに**なりません**。
> - ロックの古さやヘルスチェックの失敗を理由にロックを削除しないでください。

### 高可用性

Controller の Deployment は既定で 1 レプリカです。待機用の Controller を追加する場合は、
[controller.yaml](deploy/manifests/controller.yaml) の `replicas`（Helm チャートでは `controller.replicas`）を増やしてください。

- すべての Controller Pod から、NFS など同じ保存先にアクセスできる必要があります。
- 実行中の操作がクラッシュした後のフェイルオーバーには、[残ったロックの復旧](#残ったロックの復旧)が必要です。

### 更新

マニフェストと Helm チャートはどちらも `Recreate` 方式で更新します。複数レプリカの場合も、古い Controller Pod
をすべて停止してから新しいバージョンを起動します。

- 更新中は Controller の操作を利用できません。マウント済みのボリュームは引き続き使えます。
- 以前のロック実装から更新する場合も、この更新方式を維持してください。プロセス内のロックだけ、またはファイルロックを使う
  以前の Controller は、ディレクトリロックと協調できないため、先に停止する必要があります。
- 古い Controller のノードに到達できない場合は、新しいバージョンを起動する前に、そのノードと保留中のストレージ操作を
  フェンシングしてください。

## テスト

### 単体テスト

```sh
cargo test
```

Controller のローカルファイルのライフサイクルを対象とし、特権は不要です。

### E2E テスト

```sh
test/e2e/run.sh
```

[test/e2e/run.sh](test/e2e/run.sh) はイメージをビルドして [kind](https://kind.sigs.k8s.io/) クラスターにドライバーをデプロイし、
PVC の作成、書き込み、オンライン拡張、読み取り専用での再マウント、Multi-Mount Protection、削除までを確認します。

| 環境変数         | 効果                                                                                          |
|------------------|-----------------------------------------------------------------------------------------------|
| `DEPLOY=helm`    | マニフェストの代わりに Helm チャートをインストール                                            |
| `BACKEND=nfs`    | kind ノード上の `file://` ディレクトリの代わりに、クラスター内の NFS サーバー Pod の NFS エクスポートを保存先に使用 |
| `KEEP_CLUSTER=1` | デバッグ用にクラスターを残す                                                                  |

Docker、kind、kubectl と、ループデバイスを使えるホストカーネルが必要です。`BACKEND=nfs` ではさらに `nfs` と `nfsd`
カーネルモジュールが必要です。

## ライセンス

Apache License 2.0。[LICENSE](LICENSE) を参照してください。
