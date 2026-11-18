#!/usr/bin/env python3
"""Build original, template-grouped synthetic quality fixtures and offline LM.

These fixtures measure regression/generalization across authored templates, not
natural typing accuracy. No user logs, imported dictionaries or app text are read.
"""
from collections import Counter
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OBJECTS = "文件 方案 报告 计划 资料 问题 消息 邮件 合同 订单 发票 申请 结果 任务 数据 文档 程序 项目 表格 内容 记录 信息 设置 流程 需求 设计 参数 代码 页面 图片".split()
TRAIN_HEADS = [
    "请/尽快", "请/认真", "请/及时", "请/仔细", "请/马上", "麻烦/你/先", "麻烦/大家", "希望/你/可以",
    "我们/需要", "你们/需要", "我/正在", "他/正在", "同事/正在", "大家/正在", "我们/已经", "他们/已经",
    "今天/需要", "明天/需要", "上午/需要", "下午/需要", "下班/之前/需要", "开会/之前/需要",
    "收到/之后/请", "有空/的时候/请", "方便/的时候/请", "我们/准备", "我/打算", "他/打算",
    "请/安排/同事", "请/提醒/大家", "我/想/请/你", "我/建议/大家", "稍后/我们/会", "明天/我们/会",
    "下午/我/会", "这次/我们/可以", "你/现在/可以", "我们/应该", "大家/一起", "我/会/帮/你",
]
TRAIN_ACTIONS = ["检查", "确认", "处理", "修改", "更新", "整理", "查看", "保存", "讨论", "准备"]
DEV_TEMPLATES = [
    "这个/{}/我/还/没有/看/过", "你/能/帮/我/确认/一下/这个/{}吗", "等/我/看/完/这个/{}/再/回复/你",
    "我们/下午/一起/讨论/这个/{}吧", "关于/这个/{}/我/有/一个/建议", "这份/{}/需要/重新/检查/一遍",
    "我/刚才/已经/检查/过/这个/{}了", "请/告诉/我/这个/{}/是否/需要/修改", "你/觉得/这个/{}/怎么样",
    "我们/能不能/明天/再/讨论/这个/{}",
]
TEST_TEMPLATES = [
    "我/想/知道/这个/{}/什么时候/可以/完成", "如果/这个/{}/没有/问题/就/发/给/我",
    "今天/先/把/这个/{}/处理/好/再/回家", "你/看/完/这个/{}/以后/记得/联系/我",
    "不好意思/这个/{}/可能/还/需要/一点/时间", "麻烦/把/修改/之后/的/{}/再/给/我/看/一下",
    "我/现在/不/方便/看/这个/{}/晚点/再/说", "谢谢/你/帮/我/解决/了/这个/{}/的/问题",
    "下次/开会/我们/再/一起/讨论/这个/{}", "昨天/说/的/那个/{}/现在/已经/处理/好/了",
]

def fnv(data):
    h = 0xcbf29ce484222325
    for b in data:
        h = ((h ^ b) * 0x100000001b3) & ((1 << 64) - 1)
    return h

def readings():
    rows = {}
    for name in ("dict.txt", "extra.txt", "frost.txt"):
        for line in (ROOT / "data" / name).read_text().splitlines():
            cols = line.split("\t")
            if len(cols) >= 2 and not line.startswith("#"):
                rows.setdefault(cols[0], cols[1])
    # Authored readings for grammatical pieces; no automatic tone/polyphone guessing.
    for word, code in [ ("的", "de"), ("了", "le"), ("过", "guo"), ("吗", "ma"), ("吧", "ba"),
                       ("还", "hai"), ("那个", "na'ge"), ("之后", "zhi'hou"), ("一遍", "yi'bian") ]:
        rows[word] = code
    return rows

def code_for(text, dictionary):
    out = []
    while text:
        for size in range(min(8, len(text)), 0, -1):
            if text[:size] in dictionary:
                out.extend(dictionary[text[:size]].split("'"))
                text = text[size:]
                break
        else:
            raise ValueError("missing reading: " + text)
    return out

def grams(s):
    return {s[i:i+4] for i in range(max(1, len(s)-3))}

def corpus():
    dictionary = readings()
    fixtures = []
    for split, templates in [("dev", DEV_TEMPLATES), ("test", TEST_TEMPLATES)]:
        for group, template in enumerate(templates):
            for obj in OBJECTS:
                sentence = template.format(obj).replace("/", "")
                syls = code_for(sentence, dictionary)
                mixed = "".join(s if i % 3 == 0 else s[0] for i, s in enumerate(syls))
                fixtures.append((split, f"{split}-{group:02}", sentence, "".join(syls), "".join(s[0] for s in syls), mixed))
    held = [grams(row[2]) for row in fixtures]
    old = [line.split("\t")[0] for line in (ROOT / "tests/sentences.tsv").read_text().splitlines() if line and not line.startswith("#")]
    held += [grams(s) for s in old]
    train = []
    for head in TRAIN_HEADS:
        for action in TRAIN_ACTIONS:
            for obj in OBJECTS:
                segmented = f"{head}/{action}/{obj}"
                g = grams(segmented.replace("/", ""))
                if any(len(g & h) / max(1, len(g | h)) >= 0.65 for h in held):
                    continue
                train.append(segmented)
    # Balanced deterministic sampling rather than the first template groups.
    original = [s for s in (ROOT / "data/conversation-training.txt").read_text().splitlines() if s and not s.startswith("#")]
    original = [s for s in original if not any(len(grams(s.replace("/", "")) & h) / max(1, len(grams(s.replace("/", "")) | h)) >= 0.65 for h in held)]
    train.sort(key=lambda s: hashlib.sha256(s.encode()).digest())
    train = sorted(original + train[:2000-len(original)])
    assert len(train) == 2000 and len(set(train)) == 2000
    (ROOT / "data/office-training.txt").write_text("# Original synthetic training only; MIT; / marks word boundaries.\n" + "\n".join(train) + "\n")
    (ROOT / "tests/quality_sentences.tsv").write_text("# Synthetic authored templates; split by template group, never training examples.\n# split\tgroup\texpected\tfull\tinitials\tmixed\n" + "\n".join("\t".join(row) for row in fixtures) + "\n")
    excluded = {line.split("\t")[1] for line in (ROOT / "tests/common.tsv").read_text().splitlines()}
    words = []
    for line in (ROOT / "data/freq.txt").read_text().splitlines():
        word = line.split()[0]
        if word not in excluded and word in dictionary and 2 <= len(word) <= 6:
            words.append((dictionary[word].replace("'", ""), word))
        if len(words) == 300:
            break
    (ROOT / "tests/quality_words.tsv").write_text("# Dictionary-selected frequency samples, not independently labelled natural input.\n" + "\n".join("\t".join(row) for row in words) + "\n")
    return train

def model(train, output=None):
    tables = []
    for kind in ["C", "W"]:
        one, two, three = Counter(), Counter(), Counter()
        for sentence in train:
            tokens = [ord(c) for c in sentence.replace("/", "")] if kind == "C" else [fnv(t.encode()) for t in sentence.split("/")]
            previous = [0, 0]
            for token in tokens:
                one[token] += 1
                two[previous[1], token] += 1
                three[previous[0], previous[1], token] += 1
                previous = [previous[1], token]
        total = sum(one.values())
        if kind == 'W':
            import math
            for token,count in sorted(one.items()):
                if count >= 2: tables.append(('U',0,0,token,-math.log(count/total),count))
        contexts2, contexts3 = Counter(), Counter()
        for (a,b), count in two.items(): contexts2[a] += count
        for (a,b,c), count in three.items(): contexts3[a,b] += count
        import math
        for a,count in sorted(contexts2.items()):
            tables.append((kind,0,a,0,-min(8,math.log1p(count/4)),count))
        for (a,b),count in sorted(contexts3.items()):
            if a: tables.append((kind,a,b,0,-min(8,math.log1p(count/4)),count))
        for (a,b), count in sorted(two.items()):
            if count < 2: continue
            p1 = one[b] / total
            p2 = (count + 4*p1) / (contexts2[a] + 4)
            tables.append((kind,0,a,b,max(-8, min(8, math.log(p2/p1))), count))
        for (a,b,c), count in sorted(three.items()):
            if not a or count < 2: continue
            p1 = one[c] / total
            p2 = (two[b,c] + 4*p1) / (contexts2[b] + 4)
            p3 = (count + 4*p2) / (contexts3[a,b] + 4)
            tables.append((kind,a,b,c,max(-8, min(8, math.log(p3/p1))), count))
    # Bound both file size and runtime hash-table allocations deterministically.
    tables = sorted(tables, key=lambda r: (-r[5],r[:4]))[:199999]
    tables.sort(key=lambda r:r[:4])
    output = output or ROOT / "data/context-model.txt"
    output.write_text("".join(f"{k}\t{a}\t{b}\t{c}\t{w:.6f}\n" for k,a,b,c,w,count in tables))
    return len(tables)

if __name__ == "__main__":
    train = corpus()
    print(json.dumps({"training_sentences":len(train),"model_records":model(train),"dev_sentences":300,"test_sentences":300,"new_words":300}))
