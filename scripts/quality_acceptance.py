#!/usr/bin/env python3
"""Serial release acceptance with per-version images and isolated user state."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
from input_perf import metrics
from ibus_perf import run as bus_run
from benchmark_memory import snapshot

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def median(rows):
    return {group:{key:statistics.median(row[group][key] for row in rows) for key in rows[0][group]}
            for group in rows[0]}

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--baseline',type=Path,required=True)
    p.add_argument('--image',type=Path,required=True)
    p.add_argument('--engine',type=Path,required=True)
    p.add_argument('--probes',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--runs',type=int,default=7)
    a=p.parse_args()
    assert a.runs>=3
    out=a.output.resolve();out.mkdir(parents=True,exist_ok=True)
    image=a.image.resolve();engine=a.engine.resolve();probes=a.probes.resolve();base=a.baseline.resolve()
    report={'runs':a.runs,'sha256':{'image':sha(image),'model':sha(image.with_suffix('.lm')),
        'engine':sha(engine),'before_probe':sha(base/'input_latency'),'after_probe':sha(probes/'input_latency'),
        'baseline_image':sha(base/'lexicon.bin')},'samples':{},'routed_ibus':[],'engine_memory':[]}
    def save(): (out/'results.json').write_text(json.dumps(report,indent=2)+'\n')
    with tempfile.TemporaryDirectory(prefix='waypinyin-acceptance-') as tmp:
        env=dict(os.environ,XDG_DATA_HOME=tmp+'/data',XDG_CONFIG_HOME=tmp+'/config')
        for name,binary,dictionary in [('before',base/'input_latency',base/'lexicon.bin'),('after',probes/'input_latency',image)]:
            selected=dict(env,WAYPINYIN_LEXICON=str(dictionary))
            subprocess.run([str(binary),'bench'],env=selected,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True,timeout=180)
            rows=[]
            for i in range(a.runs):
                r=subprocess.run([str(binary),'bench'],env=selected,text=True,capture_output=True,check=True,timeout=180)
                (out/f'{name}-input-{i}.txt').write_text(r.stdout)
                rows.append(metrics(r.stdout));print(f'{name} input {i+1}/{a.runs}',flush=True)
            report['samples'][name]=rows;save()
        traces=[]
        for i in range(3):
            text=subprocess.check_output([str(probes/'input_latency'),'trace'],env=dict(env,WAYPINYIN_LEXICON=str(image)),text=True,timeout=180)
            (out/f'candidate-trace-{i}.tsv').write_text(text);traces.append(text.splitlines())
        report['determinism']={'line_counts':[len(t) for t in traces],
            'variant_lines':[i+1 for i in range(min(map(len,traces))) if len({t[i] for t in traces})>1]}
        contexts=[]
        for i in range(a.runs):
            text=subprocess.check_output([str(probes/'context_latency')],env=dict(env,WAYPINYIN_LEXICON=str(image)),text=True,timeout=180)
            (out/f'context-{i}.txt').write_text(text);contexts.append(metrics(text))
            print(f'Unicode contexts {i+1}/{a.runs}',flush=True)
        report['context_samples']=contexts
        report['context_median']=median(contexts)
        # Real isolated daemon routing, independent of the desktop input context.
        # Signal-dedup verification targets the engine directly: IBus itself
        # may suppress an initial empty update before forwarding to a client.
        # Real routed commits/UI are separately exercised by check_ibus.py.
        bus_run(engine,probes/'ibus_latency',image,'verify')
        for i in range(a.runs):
            text,_=bus_run(engine,probes/'ibus_latency',image,'bench',routed=True)
            (out/f'ibus-routed-{i}.txt').write_text(text)
            report['routed_ibus'].append(metrics(text));print(f'routed IBus {i+1}/{a.runs}',flush=True);save()
        for i in range(3):
            bus_run(engine,probes/'ibus_latency',image,'bench',observe=lambda pid: report['engine_memory'].append(snapshot(pid)))
            print(f'engine memory {i+1}/3',flush=True);save()
        if (base/'installed-engine').is_file():
            before_memory=[]
            for i in range(3):
                bus_run(base/'installed-engine',probes/'ibus_latency',base/'lexicon.bin','bench',observe=lambda pid:before_memory.append(snapshot(pid)))
            report['installed_baseline_memory']=before_memory
            report['sha256']['installed_baseline_engine']=sha(base/'installed-engine')
        life,_=bus_run(engine,probes/'ibus_latency',image,'lifecycle')
        report['lifecycle']=life.strip()
    report['median']={name:median(rows) for name,rows in report['samples'].items()}
    report['ibus_median']=median(report['routed_ibus'])
    before=report['median']['before']['short_key']['p95_ns'];after=report['median']['after']['short_key']['p95_ns']
    report['acceptance']={'ordinary_sentence_key_p95_le_10ms':report['median']['after']['sentence_key']['p95_ns']<=10_000_000,
        'short_key_p95_regression_le_20_percent':after<=before*1.2,
        'surrounding_context_sentence_p95_le_10ms':all(report['context_median'][f'context_{n}_sentence']['p95_ns']<=10_000_000 for n in (16,64)),
        'deterministic_traces':not report['determinism']['variant_lines'],
        # This full-process high-water bound is stricter than an additional
        # 256 MiB model budget and avoids subtracting incomparable workloads.
        'whole_engine_high_water_le_256MiB':max(m['VmHWM_kib'] for m in report['engine_memory'])<=256*1024,
        'model_file_le_128MiB':image.with_suffix('.lm').stat().st_size<=128*1024*1024}
    save();print(json.dumps({'median':report['median'],'acceptance':report['acceptance']},indent=2))

if __name__=='__main__':main()
