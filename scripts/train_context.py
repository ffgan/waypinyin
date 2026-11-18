#!/usr/bin/env python3
"""Train a bounded local model from an explicit Tatoeba detailed snapshot.

No implicit downloads or user-data access. The CC BY 2.0 FR source and derived
model remain local; original synthetic fixtures are maintained separately.
"""
import argparse
import bz2
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import re
import quality_corpus as qc

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--corpus', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--snapshot', default='unspecified', help='upstream snapshot date recorded in provenance')
    parser.add_argument('--expected-sha256', help='reject a different source snapshot')
    args = parser.parse_args()
    source_sha256 = hashlib.sha256(args.corpus.read_bytes()).hexdigest()
    if args.expected_sha256 and source_sha256 != args.expected_sha256:
        raise SystemExit('source snapshot checksum mismatch')
    root = qc.ROOT
    held = [l.split('\t')[2] for l in (root/'tests/quality_sentences.tsv').read_text().splitlines() if not l.startswith('#')]
    held += [l.split('\t')[0] for l in (root/'tests/sentences.tsv').read_text().splitlines() if not l.startswith('#') and l]
    held_grams = [qc.grams(s) for s in held]
    index = defaultdict(set)
    for i, grams in enumerate(held_grams):
        for gram in grams: index[gram].add(i)
    conversion = {}
    for l in (root/'data/ts-characters.txt').read_text().splitlines():
        if l and not l.startswith('#'):
            a,b = l.split('\t')[:2]
            if len(a)==1: conversion[ord(a)] = b.split()[0]
    vocabulary = {l.split()[0] for l in (root/'data/freq.txt').read_text().splitlines() if int(l.split()[1]) >= 10}
    vocabulary.update(qc.OBJECTS)
    def segment(s):
        out = []
        while s:
            size = next((n for n in range(min(8,len(s)),1,-1) if s[:n] in vocabulary),1)
            out.append(s[:size]); s=s[size:]
        return '/'.join(out)
    train = [l for l in (root/'data/office-training.txt').read_text().splitlines() if l and not l.startswith('#')]
    original = len(train)
    seen, credits, removed = set(), [], 0
    for line in bz2.open(args.corpus, 'rt'):
        fields = line.rstrip('\n').split('\t')
        if len(fields) < 6 or fields[1] != 'cmn': continue
        text = fields[2].translate(conversion)
        # Train only Chinese clauses; foreign names, numbers, URLs and long
        # paragraphs do not supply probabilities for ordinary IME sentences.
        if re.search(r'[A-Za-z0-9０-９]',text): continue
        clauses = re.findall(r'[\u4e00-\u9fff]{4,80}',text)
        accepted = False
        for sentence in clauses:
            if sentence in seen: continue
            g = qc.grams(sentence)
            candidates = set().union(*(index.get(gram,set()) for gram in g))
            if any(sentence in held[i] or held[i] in sentence or len(g & held_grams[i])/max(1,len(g | held_grams[i]))>=0.65 for i in candidates):
                removed += 1; continue
            seen.add(sentence)
            train.append(segment(sentence)); accepted=True
        if accepted: credits.append((fields[0], fields[3]))
    if not seen: raise SystemExit('no usable detailed Mandarin records; model not written')
    # Reweight original conversational/office clauses without changing holdout.
    train += train[:original] * 3
    args.output.parent.mkdir(parents=True,exist_ok=True)
    records = qc.model(train, args.output)
    body = args.output.read_bytes()
    credit_path = args.output.with_suffix('.attribution.tsv')
    credit_path.write_text('sentence_id\tusername\n'+'\n'.join('\t'.join(row) for row in credits)+'\n')
    metadata = {'source':'https://downloads.tatoeba.org/exports/per_language/cmn/cmn_sentences_detailed.tsv.bz2',
        'source_information':'https://tatoeba.org/en/downloads', 'license':'CC-BY-2.0-FR',
        'license_url':'https://creativecommons.org/licenses/by/2.0/fr/',
        'snapshot':args.snapshot, 'source_sha256':source_sha256,
        'transformation':'Traditional character mapping, Chinese clause filtering, greedy dictionary segmentation, word unigram costs and interpolated word/character backoff log-ratios with context backoff penalties; count>=2 pruning, at most 199999 records retained by frequency. Original office examples weighted fourfold.',
        'authors':'Tatoeba contributors; per-sentence IDs and supplied usernames in '+credit_path.name,
        'attribution_sha256':hashlib.sha256(credit_path.read_bytes()).hexdigest(),
        'model_body_sha256':hashlib.sha256(body).hexdigest(), 'records':records,
        'external_clauses':len(seen),'original_sentences':original,'removed_holdout_matches':removed,
        'training_sentence_sha256':hashlib.sha256('\n'.join(train).encode()).hexdigest(),
        'pipeline_sha256':{str(path.relative_to(root)):hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [Path(__file__).resolve(), root/'scripts/quality_corpus.py', root/'data/freq.txt',root/'data/ts-characters.txt',root/'data/office-training.txt',root/'tests/quality_sentences.tsv',root/'tests/sentences.tsv']},
        'distribution':'local trial only; do not export with the MIT source fixture'}
    args.output.with_suffix('.json').write_text(json.dumps(metadata,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(metadata,ensure_ascii=False))

if __name__ == '__main__': main()
