# Waypinyin

> **公开源码版 / Public source edition:** GitHub 仓库仅包含源码与原创演示词库，
> 不包含历史第三方词库及其 Git 历史。默认可构建并运行演示；完整词库通过
> `WAYPINYIN_SOURCE_DATA` 在本地提供。详见 [公开源码说明](docs/PUBLIC_SOURCE.md)。
> The public repository builds a small original demo dictionary, not the full
> daily-use corpus. See [source/data separation](docs/PUBLIC_SOURCE.md).

通过 IBus 为 GNOME Wayland 提供离线拼音输入。 [English](README.en.md)

当前为发布准备阶段。原创源码使用 MIT；第三方词库不适用本项目的 MIT 授权。
历史完整词库的来源和再分发证据尚未全部核实，因此公开版本排除了这些数据与旧历史；完整词库及其编译产物暂未获得发布放行。
详见 [发布阻塞项](docs/RELEASE.md) 和 [来源清单](data/sources.json)。

## 标准安装

需要 Rust 1.99.1（当前验证的最低版本）、Cargo、Python 3.11+、GNU make、C 编译工具和 Fontconfig 开发包。
运行需要 IBus、Wayland/X11、OpenGL/EGL、Fontconfig 及中文字体；文件选择依赖桌面 portal。

```sh
make build
./build/waypinyin --help
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" ./build/waypinyin --check-data
make install PREFIX=/usr/local DESTDIR="$PWD/build/stage"
```

`DESTDIR` 仅暂存安装文件。实际安装时去掉它并使用相应目录写权限；发行版使用 `PREFIX=/usr`。
构建与安装时保持 `PREFIX`、`DATADIR` 一致，可另外配置 `BINDIR`、`MANDIR`。
自定义前缀能否被 IBus 自动发现取决于桌面配置。安装不会更改输入源或重启服务；安装后重新登录并在 GNOME 输入源中添加 Waypinyin。
`cargo install` 单独执行不会安装词库和 IBus 注册文件。

`cargo build` 将词库写入 Cargo 的 `OUT_DIR`，不再向 profile 根目录隐式复制。
调试构建可直接加载构建词库；release 开发运行与评测需要显式设置 `WAYPINYIN_LEXICON`。
已安装程序从安装数据目录读取词库，不依赖源码或构建目录。

[验证记录](docs/VALIDATION.md) · [打包说明](docs/PACKAGING.md) · [贡献指南](CONTRIBUTING.md) · [安全报告](SECURITY.md) · [变更记录](CHANGELOG.md)

## 构建与词库管理

```sh
make build
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" ./build/waypinyin --settings
```

设置窗口可选择文件、预览记录、确认导入、启用、停用和移除词库。
将 `share/waypinyin-settings.desktop` 安装到 `~/.local/share/applications/`，并确保 `waypinyin` 在 PATH 中，即可从应用菜单打开。

支持格式：

- 搜狗 `.scel`（0x44 / 0x45 版本）及双列文本。
- 微软用户自定义短语 `.dat`（`mschxudp`）；不支持自学习 `ChsPinyinUDL.dat`。
- QQ `.qpyd`、`.qcel` 和双列文本。
- 苹果用户文本替换 `.plist`（XML 或二进制，`shortcut` / `phrase` 字段）。保留原缩写，仅精确匹配。
- 通用 TSV、Rime `.dict.yaml`：`词语<TAB>ni hao<TAB>可选频次`，或 `ni'hao<TAB>词语`。支持 UTF-8、带 BOM 的 UTF-16LE，以及 GBK。

导入范围为以上文件版本，并不包含厂商系统词库或云候选。当前导入忽略外部频次，使用保守权重，避免不同来源的频次量纲影响常用词排序。文本导出中其他私有布局会报告跳过，建议转为 TSV。

导入文件保存在 `$XDG_DATA_HOME/waypinyin/dictionaries/`，默认 `~/.local/share/waypinyin/dictionaries/`。同名导入替换已有词库；不同来源相同词在候选中去重。词库独立于个人学习日志，没有 4000 词上限。修改在下次以字母开始输入时生效，当前组合不受影响。导入词不计作个人选择记录。

完整精确候选排在前缀候选之前；前缀仍可部分上屏。个人学习的词频倍数最多为九倍，原有衰减与删除操作继续有效。新造词和旧版无基础频次的个人记录使用当前拼音的基础权重。

## 验证

```sh
cargo test
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" cargo run --release --example quality
WAYPINYIN_FIXTURES=/path/to/native/dictionaries cargo test native_dictionaries -- --ignored --nocapture
```

`tests/common.tsv` 固定保存按现有日常词频选取的 200 组常用词，验证首选和第一页不退步，不代表自然语言输入的总体准确率。`quality` 输出命中数和查询 P95；性能比较应使用相同词库镜像、构建优化及缓存条件。

原生格式额外用深蓝词库转换项目的两份 scel、一份 qcel、一份 qpyd 文件验证；外部文件不随本项目分发。微软使用构造记录验证，苹果使用 plist 结构验证，仍需各平台实际导出文件做进一步兼容验证。

词库数据来源与许可见 `data/NOTICE`。二进制布局参考：
https://github.com/studyzy/imewlconverter

尚未完成：新的默认词库数据更新与跨来源词频校准、更广泛的真实输入评测、GNOME 中完整的人工选词验收。

## 整句与简拼

长句使用有界 beam search 比较整个输入的词语切分，支持全拼、声母简拼、zh/ch/sh 和混合拼写；保留前三种组句结果。完整词匹配继续优先于前缀，简拼整词也可直接按空格上屏。

`data/sentences.txt` 是人工编写的日常语境样本，仅用于轻量的字符搭配评分，不收集输入记录。已有词频用于词语概率，语境评分用于减少“汽车服务”和“吃饭”等简拼歧义。八条路径的 beam 和搜索预算限制耗时；超出预算或无法组句时保留原有词语候选。高度歧义的纯简拼仍可能需要补写部分全拼。

```sh
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" cargo run --release --example sentence_quality
```

该评测包含 `tests/sentences.tsv` 中 25 句的全拼、简拼和混合拼写，共 75 例（含约 20 字的多分句长句），统计真实 Session 首选、第一页及每次按键的耗时。它属于人工回归集，不能视作任意长句的自然语言准确率。

## 模糊拼音与输入设置

运行 `waypinyin --settings`，在“输入体验”中设置模糊拼音总开关和各音组，点击“保存输入设置”。默认开启 z/zh、c/ch、s/sh、an/ang、en/eng、in/ing；n/l、f/h、r/l 默认关闭。各组双向匹配，可组合使用，例如开启对应规则后，`zan` 可匹配 `zhang`。精确候选仍优先，关闭模糊拼音不影响简拼和 ü 的等价拼写。

设置保存在 `$XDG_CONFIG_HOME/waypinyin/config.toml`（默认 `~/.config/waypinyin/config.toml`），下一次开始组合输入时生效。当前组合保持原设置。“恢复默认”后仍需点击保存。配置损坏时回退默认值并在标准错误中报告原因。精确快捷短语不参与模糊匹配。

支持默认开启的 ou/uo 模糊音组（配置键 `ou_uo`），例如 `sou/suo`、`shou/shuo`；与 s/sh 同时开启时，这四种拼写可互相匹配。可在设置窗口独立关闭，已有配置缺少该键时采用开启默认值。


## 个人学习记录与数据安全

个人学习记录现在遵循 `$XDG_DATA_HOME/waypinyin/user.txt`，默认位置不变。
切换到自定义 XDG 数据目录后，仅在新文件不存在时复制旧位置的记录，保留旧文件；迁移失败会报告并停止本次会话的学习写入。
空值或相对 XDG 路径按未设置处理。词库、设置窗口需要有效的 HOME 或绝对 XDG 目录。
个人记录包含本地选词信息，提交问题报告时请勿上传。

编译词库是可信安装数据，不是导入格式。不要在运行期间改写或截断已映射的词库。
当前镜像使用本机小端记录布局，应为目标架构生成；暂不支持大端或交叉编译。
