#!/usr/bin/env python3
"""Verify aggregate summaries against an existing disposable large-file fixture."""
import argparse,json,time
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--url',default='http://localhost:13001');p.add_argument('--token-file',required=True);p.add_argument('--chromium',required=True);p.add_argument('--fixture-report',required=True);p.add_argument('--report',default='/tmp/geoledger-summary-report.json')
a=p.parse_args();fixture=json.loads(Path(a.fixture_report).read_text());token=json.loads(Path(a.token_file).read_text())[0]['token'];report={'features':fixture['expected_features'],'requests':[],'page_errors':[]};project=fixture['project'];workspace=fixture['workspace']['id'];revision=fixture['publication']['revision']
with sync_playwright() as pw:
 browser=pw.chromium.launch(headless=True,executable_path=a.chromium,args=['--no-sandbox','--disable-dev-shm-usage'])
 page=browser.new_page(viewport={'width':1600,'height':1000});page.on('pageerror',lambda e:report['page_errors'].append(str(e)))
 page.goto(a.url+'/login');page.get_by_label('访问令牌',exact=True).fill(token);page.get_by_role('button',name='进入控制台').click();page.wait_for_url('**/projects')
 def record(request):
  if '/api/console' not in request.url or request.method!='POST':return
  response=request.response();body=request.post_data_json or {}
  report['requests'].append({'action':body.get('action'),'status':response.status,'ms':round(request.timing['responseEnd']-request.timing['requestStart'],2)})
 page.on('requestfinished',record)
 report['samples']=[]
 for action in ['workspaceSummary','commitSummary']:
  for _ in range(3):
   body={'action':action,'project':project,**({'workspace':workspace} if action=='workspaceSummary' else {'revision':revision})}
   result=page.evaluate("""async body=>{const t=performance.now();const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});const text=await r.text();return {status:r.status,ms:performance.now()-t,bytes:new TextEncoder().encode(text).length,data:JSON.parse(text)}}""",body)
   assert result['status']==200,result
   assert int(result['data']['total']['added'])==report['features'],result
   assert int(result['data']['total']['deleted'])==int(result['data']['total']['modified'])==0
   assert len(result['data']['datasets'])==1
   assert result['data']['datasets'][0]['id']==fixture['dataset']
   report['samples'].append({'action':action,**result})
 for section in ['workspaces','history']:
  started=time.monotonic();page.goto(a.url+'/'+section+'?project='+project)
  summary=page.get_by_label('按数据集统计要素变更').filter(has_text=str(report['features'])).first
  expect(summary).to_be_visible(timeout=15000)
  report[section+'_visible_seconds']=round(time.monotonic()-started,3)
  assert 'LUCC' in summary.inner_text()
  assert '统计失败' not in page.locator('body').inner_text()
  page.screenshot(path=a.report+'.'+section+'.png',full_page=True)
 assert not report['page_errors'],report['page_errors']
 assert not [r for r in report['requests'] if r['action'] in ['diff','commit','features']],report['requests']
 report['success']=True;Path(a.report).write_text(json.dumps(report,ensure_ascii=False,indent=2));print(json.dumps(report,ensure_ascii=False,indent=2));browser.close()
