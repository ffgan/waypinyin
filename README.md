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

需要 Rust 1.89.0（当前验证的最低版本）、Cargo、Python 3.11+、GNU make、C 编译工具和 Fontconfig 开发包。
运行需要 IBus、Wayland/X11、OpenGL/EGL、Fontconfig 及中文字体；文件选择依赖桌面 portal。

```sh
make build
./build/waypinyin --help
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" ./build/waypinyin --check-data
make install PREFIX=/usr/local DESTDIR="$PWD/build/stage"
```

`make install` 只安装已生成的产物，不会编译或访问网络；请先以最终安装路径运行 `make build`。

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

长句使用有界 beam search 比较整个输入的词语切分，支持全拼、声母简拼、zh/ch/sh 和混合拼写；基础搜索保留前三种组句结果，可选模型的增强搜索最多保留五种。完整词匹配继续优先于前缀，简拼整词也可直接按空格上屏。

`data/sentences.txt` 是人工编写的日常语境样本，仅用于轻量的字符搭配评分，不收集输入记录。已有词频用于词语概率，语境评分用于减少“汽车服务”和“吃饭”等简拼歧义。基础搜索保留八条路径，增强搜索保留四条；固定搜索预算限制耗时，无法组句时保留原有词语候选。高度歧义的纯简拼仍可能需要补写部分全拼。

```sh
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" cargo run --release --example sentence_quality
```

该评测包含 `tests/sentences.tsv` 中 25 句的全拼、简拼和混合拼写，共 75 例（含约 20 字的多分句长句），统计真实 Session 首选、第一页及每次按键的耗时。它属于人工回归集，不能视作任意长句的自然语言准确率。

### 可选离线上下文模型

模型使用词语一至三元统计和字符搭配，针对基础评分把握较低的整句重新搜索；已有搭配充分支持的读法保持稳定。词频编译按共同词校准来源尺度，并保留常用词锚点及一半原始先验，避免人工权重被当作真实频次直接抹去。末尾未完成音节可以结合语境补全；基础搜索失败时，模型搜索可使用模糊拼音规则和整句至多一次拼写编辑，不跨分隔符修改拼写。

在设置的“输入体验”中控制“上下文增强”和“使用光标前文本”，配置键分别为 `contextual`、`surrounding`，默认开启，有效模型缺失时仍使用基础引擎。支持周边文本的应用可以提供光标前最多 64 个字符；没有周边文本时使用本输入法已确认的当前句内容。周边文本不写日志、不用于训练、不发送到预加载线程，失焦、重置或进入私密输入时清除。单词上下文加权最多两倍，个人学习继续使用原有至多九倍的衰减权重。

模型为词库旁的可选 `lexicon.lm`，包含版本、词库指纹及内容校验；缺失、损坏或不匹配时诊断并回退。原始演示构建不自动训练或下载模型。使用明确提供的详细普通话语料快照在本机训练：

```sh
python3 scripts/train_context.py --corpus /absolute/path/cmn_sentences_detailed.tsv.bz2 \
  --output "$PWD/build/context-model.txt"
WAYPINYIN_MODEL_BODY="$PWD/build/context-model.txt" make build PREFIX="$HOME/.local"
make install PREFIX="$HOME/.local"
```

本地完整试用模型使用 Tatoeba CC BY 2.0 FR 语料及 2,000 条原创训练句；训练工具保留来源校验值、句子 ID 和作者名，剔除验收句及近似句。安装保留模型署名和来源元数据；该模型不随 MIT 演示源码发行。详见 `LICENSES/Tatoeba-model-NOTICE.txt`。

扩展评测包含 300 条按词频选择的词样本、按模板分开的 300 句开发集与 300 句验收集，各句覆盖三种输入方式。它是合成回归评测，不代表真实输入准确率。运行 `quality_suite` 或 `scripts/evaluate_quality.py` 可保存候选、首选／前 5 项命中及按键耗时；后者需要保留的基线候选轨迹。

完整模型的本机验收结果、性能边界与回滚方法见[候选质量验收记录](docs/CANDIDATE_QUALITY.md)。

## 模糊拼音与输入设置

运行 `waypinyin --settings`，在“输入体验”中设置模糊拼音总开关和各音组，点击“保存输入设置”。默认开启 z/zh、c/ch、s/sh、an/ang、en/eng、in/ing；n/l、f/h、r/l 默认关闭。各组双向匹配，可组合使用，例如开启对应规则后，`zan` 可匹配 `zhang`。精确候选仍优先，关闭模糊拼音不影响简拼和 ü 的等价拼写。

设置保存在 `$XDG_CONFIG_HOME/waypinyin/config.toml`（默认 `~/.config/waypinyin/config.toml`），下一次开始组合输入时生效。当前组合保持原设置。“恢复默认”后仍需点击保存。配置损坏时回退默认值并在标准错误中报告原因。精确快捷短语不参与模糊匹配。

支持默认开启的 ou/uo 模糊音组（配置键 `ou_uo`），例如 `sou/suo`、`shou/shuo`；与 s/sh 同时开启时，这四种拼写可互相匹配。可在设置窗口独立关闭，已有配置缺少该键时采用开启默认值。


## 个人学习记录与数据安全

个人学习记录现在遵循 `$XDG_DATA_HOME/waypinyin/user.txt`，默认位置不变。
切换到自定义 XDG 数据目录后，仅在新文件不存在时复制旧位置的记录，保留旧文件；迁移失败会报告并停止本次会话的学习写入。
空值或相对 XDG 路径按未设置处理。词库、设置窗口需要有效的 HOME 或绝对 XDG 目录。
个人记录包含本地选词信息，提交问题报告时请勿上传。

现在还会在候选确认上屏后，统计实际使用的拼音/简拼输入码及次数，例如 `nihao` 和 `nh` 分别计数；未确认草稿、取消输入和回车提交的未转换原文不计入。重复至少两次的常用输入会优先预加载完整结果及前缀，最多保留 128 项活跃统计，选取前 16 项预热；已有用户词记录也能直接提供预热线索。热度按现有学习计数时钟每 512 次递减一半，预加载不固定候选排名。

统计沿用本地 `user.txt`，不上传。日志采用追加写入，启动时按存量压缩，历史行数可能多于活跃项数。应用标记为密码、PIN、私密或隐藏的输入上下文会停用学习和新的预加载请求；涉及此类上下文的当前组合也不会在切回普通输入框后补记。新建日志和压缩临时文件使用 `0600` 权限。


编译词库是可信安装数据，不是导入格式。不要在运行期间改写或截断已映射的词库。
当前镜像使用本机小端记录布局，应为目标架构生成；暂不支持大端或交叉编译。


## Linux 性能

长句搜索现在缓存满额候选路径中的最差项，先按分数淘汰无法进入候选集合的路径，再做字符串去重；同时预计算语境字符对权重，并对搜索内部的整数键使用专用哈希。保留原有 8 路 beam、16 项待选路径、60,000 次搜索预算及同分顺序，参考算法继续验证候选和读音完全一致。

IBus 适配层分别跟踪预编辑文本和候选表，只发送发生变化的部分，减少移动光标等操作的重复消息。整句搜索在单次查询内复用遍历栈和已访问状态表的分配空间，保留原有搜索范围和排序规则。新增私有 D-Bus 对比探针，以及默认关闭的 `WAYPINYIN_TRACE_LATENCY=1` 分段耗时诊断；诊断只输出耗时统计，不记录输入内容。复现方法与测量范围见 [性能说明](docs/PERFORMANCE.md)。

词库通过只读 `mmap` 共享内核页缓存，启动校验时允许顺序预读，查询时切换为随机访问提示。导入词库和输入配置通过 `inotify` 跟踪修改，减少开始输入时的目录扫描和配置文件读取，并保留定期核对与异常回退。候选引擎复用音节切分、共享不可变查询计划，并减少整句搜索和短语组合的临时分配；候选搜索范围与顺序规则保持不变。测量方法和本机结果见 [性能说明](docs/PERFORMANCE.md)。


首屏只生成当前页的 5 个候选，其余候选在翻页时继续生成。IBus 启动和焦点进入时会后台预热字母首屏，并按当前候选读音预测下一键；预加载命中时直接共享首屏数据。缓存最多 128 项，采用约 8 MiB 的自有数据预算；按键路径不等待后台任务，个人学习和导入词仍实时合并。详细测量见 [性能说明](docs/PERFORMANCE.md)。
