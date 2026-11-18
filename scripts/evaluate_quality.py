#!/usr/bin/env python3
"""Compare frozen candidate traces; report strict synthetic holdout metrics."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--image',type=Path,required=True)
    p.add_argument('--split',choices=['dev','test'],required=True)
    p.add_argument('--baseline',type=Path,required=True,help='frozen expanded.tsv')
    p.add_argument('--suite',type=Path,default=ROOT/'target/release/examples/quality_suite')
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    fixture=ROOT/'tests/quality_sentences.tsv'
    all_rows=[l.split('\t') for l in fixture.read_text().splitlines() if l and not l.startswith('#')]
    base=[l.split('\t') for l in args.baseline.read_text().splitlines()]
    assert len(base)==len(all_rows)*3
    with tempfile.TemporaryDirectory(prefix='waypinyin-quality-') as tmp:
        env=dict(os.environ, XDG_DATA_HOME=tmp+'/data',XDG_CONFIG_HOME=tmp+'/config',WAYPINYIN_LEXICON=str(args.image.resolve()))
        result=subprocess.run([str(args.suite.resolve()),str(fixture),args.split],env=env,text=True,capture_output=True,check=True)
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.with_suffix('.tsv').write_text(result.stdout)
    actual=[l.split('\t') for l in result.stdout.splitlines()]
    selected=[(i,r) for i,r in enumerate(all_rows) if r[0]==args.split]
    metrics={}
    for mode,name in enumerate(['full','initials','mixed']):
        baseline_first=baseline_page=first=page=0
        for k,(i,row) in enumerate(selected):
            before=base[i*3+mode]
            after=actual[k*3+mode]
            assert row[3+mode]==before[0]==after[0]
            accepted=set(row[2].split('|'))
            candidates=before[1].split(' | ')
            baseline_first+=candidates[0] in accepted
            baseline_page+=bool(accepted & set(candidates[:5]))
            first+=int(after[2]);page+=int(after[3])
        metrics[name]={'cases':len(selected),'baseline_top1':baseline_first,'top1':first,'baseline_top5':baseline_page,'top5':page}
    old_errors=sum(m['cases']-m['baseline_top1'] for m in metrics.values())
    errors=sum(m['cases']-m['top1'] for m in metrics.values())
    report={'split':args.split,'fixture_sha256':sha(fixture),'baseline_trace_sha256':sha(args.baseline),
        'image_sha256':sha(args.image),'model_sha256':sha(args.image.with_suffix('.lm')),
        'suite_sha256':sha(args.suite),'metrics':metrics,
        'relative_top1_error_reduction':1-errors/old_errors,
        'acceptance':{'error_reduction_at_least_20_percent':errors<=old_errors*0.8,'no_category_top5_regression':all(m['top5']>=m['baseline_top5'] for m in metrics.values())},
        'limitations':'Original template-grouped synthetic samples; exact answers; not a natural-input accuracy estimate. No user dictionaries or surrounding text used.',
        'stderr':result.stderr.strip()}
    args.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(report,ensure_ascii=False))

if __name__=='__main__':main()
