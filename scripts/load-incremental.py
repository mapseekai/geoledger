#!/usr/bin/env python3
"""Bounded concurrent CRUD soak against a disposable existing benchmark dataset."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import math
import os
from pathlib import Path
import time
import urllib.error
import urllib.request
import uuid


def percentile(values, fraction):
    return sorted(values)[max(0, math.ceil(len(values)*fraction)-1)] if values else None


def statistics(samples, limit):
    summary = {op: {'count': len(v), 'p95_ms': percentile(v, .95), 'p99_ms': percentile(v, .99), 'max_ms': max(v)} for op, v in samples.items()}
    passed = all(v['p95_ms'] <= limit for op, v in summary.items() if op.rsplit('/', 1)[-1] in ('save', 'publish', 'replay'))
    return summary, passed


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--manifest',type=Path,required=True)
    p.add_argument('--feature',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--seconds',type=int,default=300)
    p.add_argument('--workers',type=int,default=20)
    p.add_argument('--p95-ms',type=float,default=5000)
    a=p.parse_args()
    if os.environ.get('GL_BENCH_DISPOSABLE')!='1': p.error('disposable fixture required')
    if not 1<=a.workers<=100 or not 1<=a.seconds<=86400 or a.p95_ms<=0:p.error('invalid load limits')
    manifest=json.loads(a.manifest.read_text())
    with a.feature.open() as f: original=json.loads(next(f))
    token=json.loads(Path(os.environ['GL_TOKEN_FILE']).read_text())[0]['token']
    base=os.environ['GL_BASE_URL']
    deadline=time.monotonic()+a.seconds
    def worker(index):
        timings={}; errors=[]; cycles=0
        def call(op,body,missing=False):
            body['project']=manifest['project']
            route = 'publish' if op == 'replay' else op
            req=urllib.request.Request(base+'/api/v1/'+route,data=json.dumps(body).encode(),headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'})
            started=time.monotonic()
            try:
                with urllib.request.urlopen(req,timeout=30) as r:result=json.load(r)
            except urllib.error.HTTPError as error:
                if not missing or error.code!=404: raise
                result=None
            elapsed=(time.monotonic()-started)*1000
            timings.setdefault(op,[]).append(elapsed)
            if op in ('save','publish','replay'): timings.setdefault(f'{action}/{count}/{op}',[]).append(elapsed)
            return result
        try:
            while time.monotonic()<deadline:
                count=(10,100,500)[(index+cycles)%3]
                prefix=uuid.uuid4().hex
                features=[]
                for n in range(count):
                    value=json.loads(json.dumps(original));value['id']=f'{prefix}-{n}';features.append(value)
                for action in ('add','modify','delete'):
                    stage_started=time.monotonic()
                    workspace=call('create_workspace',{})['workspace'];version=0
                    for offset in range(0,count,100):
                        edits=[]
                        for feature in features[offset:offset+100]:
                            if action=='modify': feature['properties']['attributes']['soak_revision']=cycles
                            edits.append({'dataset':manifest['dataset'],'feature_id':feature['id'],'feature':None if action=='delete' else feature})
                        version=call('save',{'workspace':workspace,'expected_workspace_version':version,'edits':edits})['version']
                    request={'workspace':workspace,'expected_workspace_version':version,'request_id':str(uuid.uuid4()),'message':'soak '+action}
                    receipt=call('publish',request)
                    timings.setdefault(f'{action}/{count}/total',[]).append((time.monotonic()-stage_started)*1000)
                    assert call('replay',request)==receipt, 'publication replay mismatch'
                    actual=call('features',{'dataset':manifest['dataset'],'feature_id':features[0]['id']},missing=action=='delete')
                    if action=='delete': assert actual is None, 'deleted feature remains'
                    else:
                        assert actual['properties']==features[0]['properties'] and actual['geometry']==features[0]['geometry'], 'readback mismatch'
                cycles+=1
        except Exception as error: errors.append(type(error).__name__+': '+str(error))
        return timings,errors,cycles
    with ThreadPoolExecutor(max_workers=a.workers) as pool:results=list(pool.map(worker,range(a.workers)))
    samples={};errors=[];cycles=0
    for timings,failures,count in results:
        errors.extend(failures);cycles+=count
        for op,values in timings.items():samples.setdefault(op,[]).extend(values)
    summary, latency_passed = statistics(samples, a.p95_ms)
    passed=not errors and all(count>=1 for _,_,count in results) and latency_passed
    a.output.write_text(json.dumps({'passed':passed,'workers':a.workers,'seconds':a.seconds,'cycles':cycles,'operations':summary,'errors':errors,'p95_limit_ms':a.p95_ms},indent=2))
    if not passed:raise SystemExit('incremental soak failed; inspect report')

if __name__=='__main__':main()
