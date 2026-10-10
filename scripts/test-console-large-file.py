#!/usr/bin/env python3
"""Measure real-file upload, publication, paged queries and map display in an isolated service."""
import argparse,json,time,uuid,hashlib
from collections import Counter
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--url',default='http://localhost:13001');p.add_argument('--token-file',required=True);p.add_argument('--chromium',required=True);p.add_argument('--geojson',required=True);p.add_argument('--report',default='/tmp/geoledger-large-file-report.json');p.add_argument('--timeout',type=int,default=900)
p.add_argument('--resume-report',action='store_true',help='Recheck published queries from the existing report without importing again')
a=p.parse_args();token=json.loads(Path(a.token_file).read_text())[0]['token'];report={'file':a.geojson,'bytes':Path(a.geojson).stat().st_size,'save_requests':[],'query_requests':[],'page_errors':[]}
def canonical(value):
 if isinstance(value,float) and value.is_integer():return int(value)
 if isinstance(value,list):return [canonical(v) for v in value]
 if isinstance(value,dict):return {k:canonical(v) for k,v in value.items()}
 return value
def signature(feature):
 return hashlib.sha256(json.dumps(canonical({'properties':feature.get('properties') or {},'geometry':feature.get('geometry')}),sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
with open(a.geojson) as source: original=json.load(source)
source_signatures=Counter(signature(f) for f in original['features']);report['expected_features']=len(original['features']);del original
expected={}
if a.resume_report:
 report=json.loads(Path(a.report).read_text());report['previous_attempt_seconds']=report.pop('total_seconds',None)
 for key in ['error','screen_text','success']:report.pop(key,None)
 report['query_recheck']=True
t0=time.monotonic()
def elapsed():return round(time.monotonic()-t0,3)
def checkpoint():Path(a.report).write_text(json.dumps(report,ensure_ascii=False,indent=2))
with sync_playwright() as playwright:
 browser=playwright.chromium.launch(headless=True,executable_path=a.chromium,args=['--no-sandbox','--disable-dev-shm-usage'])
 page=browser.new_page(viewport={'width':1600,'height':1000});page.on('pageerror',lambda e:report['page_errors'].append(str(e)))
 def response(r):
  if '/api/console' not in r.url and '/items' not in r.url:return
  body=(r.request.post_data_json or {}) if r.request.method=='POST' else {'action':'features'};action=body.get('action')
  if action in ['save','features']:
   entry={'elapsed':elapsed(),'status':r.status,'duration_ms':round(r.request.timing['responseEnd']-r.request.timing['requestStart'],2)}
   if action=='save':
    entry['features']=len(body.get('edits',[]));report['save_requests'].append(entry)
    if r.status==200:
     for edit in body.get('edits',[]):expected[edit['featureId']]=signature(json.loads(edit['feature']))
     try:entry['topology_warnings']=len(r.json().get('warnings',[]))
     except Exception:pass
   else:report['query_requests'].append(entry)
   if r.status>=400:
    try:entry['error']=r.json()
    except Exception:pass
   checkpoint()
 page.on('requestfinished',lambda request: response(request.response()) if request.response() else None)
 page.goto(a.url+'/login');page.get_by_label('访问令牌',exact=True).fill(token);page.get_by_role('button',name='进入控制台').click();page.wait_for_url('**/projects')
 def call(body):
  r=page.evaluate('''async body=>{const t=performance.now();const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});return {status:r.status,data:await r.text(),ms:performance.now()-t}}''',body)
  if r['status']!=200:raise RuntimeError(json.dumps(r,ensure_ascii=False))
  return json.loads(r['data'])
 try:
  if a.resume_report:
   project=report['project'];dataset=report['dataset'];publication=report['publication']
  else:
   project=call({'action':'createProject','name':'LUCC load test '+uuid.uuid4().hex[:8]})['id'];report['project']=project
   page.goto(a.url+'/datasets?project='+project)
   page.get_by_role('button',name='创建数据集',exact=True).click()
   page.get_by_label('名称',exact=True).fill('LUCC')
   page.get_by_role('combobox',name='数据来源',exact=True).click()
   page.get_by_role('option',name='上传 GeoJSON 文件',exact=True).click()
   page.get_by_label('GeoJSON 文件',exact=True).set_input_files(a.geojson,timeout=240000)
   page.evaluate('window.__importTicks=0;window.__importTimer=setInterval(()=>window.__importTicks++,50)')
   started=time.monotonic();page.get_by_role('button',name='创建',exact=True).click()
   deadline=time.monotonic()+a.timeout
   while time.monotonic()<deadline:
    if page.get_by_role('dialog').count()==0:break
    alerts=page.get_by_role('dialog').get_by_role('alert')
    if alerts.count() and alerts.first.inner_text().strip():raise RuntimeError('Upload: '+alerts.first.inner_text())
    page.wait_for_timeout(1000)
   else:raise RuntimeError('Upload exceeded test timeout')
   dataset=call({'action':'datasets','project':project})[0]['id'];report['dataset']=dataset
   report['import_ui_timer_ticks']=page.evaluate('clearInterval(window.__importTimer);window.__importTicks')
   report['upload_seconds']=round(time.monotonic()-started,3);report['topology_warning_count']=sum(r.get('topology_warnings',0) for r in report['save_requests']);assert Counter(expected.values())==source_signatures,'Upload changed source properties or geometry';checkpoint();print('Uploaded in',report['upload_seconds'],'seconds',flush=True)
   workspace=call({'action':'workspaces','project':project})[0];report['workspace']=workspace
   started=time.monotonic();page.locator('.map-progress').wait_for(state='hidden',timeout=240000);expect(page.locator('.map-canvas canvas')).to_be_visible(timeout=30000)
   report['initial_display_seconds']=round(time.monotonic()-started,3);report['initial_rows']=page.locator('.feature-row').count()
   started=time.monotonic();loads=0
   while page.get_by_role('button',name='继续加载',exact=True).count():
    page.get_by_role('button',name='继续加载',exact=True).click(timeout=240000)
    page.locator('.map-progress').wait_for(state='hidden',timeout=240000);loads+=1
   report['all_display_seconds']=round(time.monotonic()-started,3);report['display_load_batches']=loads+1
   report['display_footer']=page.locator('.gis-panel-foot').first.inner_text()
   assert str(report['expected_features']) in report['display_footer'],'Display omitted features'
   search=page.get_by_role('searchbox',name='搜索要素');selected=next(iter(expected));search.fill(selected)
   expect(page.locator('.feature-row')).to_have_count(1);page.locator('.feature-row').click();expect(page.locator('.feature-row')).to_have_attribute('aria-current','true')
   search.fill('');report['display_search_and_selection']=True
   page.screenshot(path=a.report+'.png',full_page=True)
   started=time.monotonic();publication=call({'action':'publish','project':project,'workspace':workspace['id'],'version':workspace['version'],'requestId':str(uuid.uuid4()),'message':'LUCC full file test'});report['publication_seconds']=round(time.monotonic()-started,3);report['publication']=publication;checkpoint()
  started=time.monotonic();count=0;after=None;seen=set();pages=0;revision=None;returned=Counter()
  while True:
   body={'action':'features','project':project,'dataset':dataset,'limit':100}
   if after:body['after']=after
   if revision:body['revision']=revision
   data=call(body);revision=data['revision'];pages+=1
   for feature in data['features']:
    assert feature['id'] not in seen;seen.add(feature['id']);digest=signature(feature);returned[digest]+=1;count+=1
    if expected:assert digest==expected[feature['id']],'Query changed feature '+feature['id']
   after=data.get('nextAfter')
   if not after:break
  report['full_query']={'features':count,'pages':pages,'seconds':round(time.monotonic()-started,3)}
  assert count==publication['changes'] or str(count)==str(publication['changes'])
  assert count==report['expected_features'];assert returned==source_signatures,'Query differs from source';report['exact_geometry_and_properties']=True
  assert not report['page_errors'],report['page_errors']
  report['success']=True
 except Exception as e:
  report['success']=False;report['error']=str(e)
  try:report['screen_text']=page.locator('body').inner_text()[-5000:];page.screenshot(path=a.report+'.png',full_page=True)
  except Exception:pass
 finally:
  report['total_seconds']=elapsed();checkpoint();print(json.dumps({k:v for k,v in report.items() if k not in ['screen_text','save_requests','query_requests']},ensure_ascii=False,indent=2),flush=True);browser.close()
 if not report.get('success'):raise SystemExit(1)
