#!/usr/bin/env python3
"""Exercise GeoJSON collection responses, snapshot links, and exact numbers in a disposable console."""
import argparse,json,time,uuid
from pathlib import Path
from urllib.parse import urljoin
from playwright.sync_api import sync_playwright,expect
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--url',default='http://localhost:13001');p.add_argument('--token-file',required=True);p.add_argument('--chromium',required=True);p.add_argument('--fixture-report');p.add_argument('--report',default='/tmp/geoledger-features-report.json');a=p.parse_args()
report={'page_errors':[]};token=json.loads(Path(a.token_file).read_text())[0]['token']
with sync_playwright() as pw:
 browser=pw.chromium.launch(headless=True,executable_path=a.chromium,args=['--no-sandbox','--disable-dev-shm-usage']);context=browser.new_context(viewport={'width':1600,'height':1000});page=context.new_page();page.on('pageerror',lambda e:report['page_errors'].append(str(e)))
 page.goto(a.url+'/login');page.get_by_label('访问令牌',exact=True).fill(token);page.get_by_role('button',name='进入控制台').click();page.wait_for_url('**/projects')
 def call(body):
  r=context.request.post(a.url+'/api/console',data=body,headers={'Origin':a.url});assert r.status==200,r.text();return json.loads(r.text())
 def get(url,status=200):
  r=context.request.get(urljoin(a.url,url));assert r.status==status,(r.status,r.text())
  if status==200:assert r.headers['content-type'].startswith('application/geo+json')
  return json.loads(r.text())
 project=call({'action':'createProject','name':'GeoJSON responses '+uuid.uuid4().hex[:8]})['id'];dataset=call({'action':'createDataset','project':project,'name':'Exact values','geometryType':'point'})['id'];workspace=call({'action':'createWorkspace','project':project})['id']
 def edit(i):return {'dataset':dataset,'featureId':str(i),'feature':'{"type":"Feature","id":"'+str(i)+'","properties":{"exact":18446744073709551615},"geometry":{"type":"Point","coordinates":[1,2]}}'}
 call({'action':'save','project':project,'workspace':workspace,'version':'0','edits':[edit(i) for i in range(3)]})
 base=f'/api/projects/{project}/collections/{dataset}/items'
 draft=get(base+'?workspace='+workspace+'&limit=1');nextdraft=next(l['href'] for l in draft['links'] if l['rel']=='next')
 call({'action':'save','project':project,'workspace':workspace,'version':'1','edits':[edit(3)]});get(nextdraft,409)
 publication=call({'action':'publish','project':project,'workspace':workspace,'version':'2','requestId':str(uuid.uuid4()),'message':'collections'})
 first=get(base+'?limit=2');assert first['type']=='FeatureCollection';assert first['numberReturned']==2;assert first['features'][0]['properties']['exact']==18446744073709551615
 post=call({'action':'features','project':project,'dataset':dataset,'limit':2});assert post['type']=='FeatureCollection';assert post['features']==first['features']
 nexturl=next(l['href'] for l in first['links'] if l['rel']=='next');assert 'revision=1' in nexturl
 other=call({'action':'createWorkspace','project':project})['id'];call({'action':'save','project':project,'workspace':other,'version':'0','edits':[edit(4)]});call({'action':'publish','project':project,'workspace':other,'version':'1','requestId':str(uuid.uuid4()),'message':'new head'})
 ids={f['id'] for f in first['features']}
 while nexturl:
  data=get(nexturl);assert data['revision']=='1';ids.update(f['id'] for f in data['features']);nexturl=next((l['href'] for l in data['links'] if l['rel']=='next'),None)
 assert ids=={'0','1','2','3'}
 assert get(base+'?bbox=10,10,20,20')['features']==[]
 for query in ['limit=1001','limit=1&limit=2','unknown=x','bbox=1,2,3']:get(base+'?'+query,400)
 anonymous=browser.new_context();assert anonymous.request.get(a.url+base).status==401;anonymous.close()
 page.goto(a.url+'/datasets?project='+project);page.get_by_role('button',name='Exact values',exact=True).click();expect(page.locator('.feature-row')).to_have_count(5,timeout=20000);page.locator('.feature-row').first.click();expect(page.locator('.feature-row').first).to_have_attribute('aria-current','true')
 page.screenshot(path=a.report+'.png',full_page=True)
 report.update(exact_numbers=True,pinned_pagination=True,stale_workspace_rejected=True,bbox=True,invalid_parameters_rejected=True,authentication=True,map_and_list=True)
 if a.fixture_report:
  fixture=json.loads(Path(a.fixture_report).read_text());url=f"/api/projects/{fixture['project']}/collections/{fixture['dataset']}/items?limit=100";count=0;pages=0;seen=set();started=time.monotonic()
  while url:
   data=get(url);assert data['type']=='FeatureCollection';assert data['numberReturned']==len(data['features']);pages+=1
   for f in data['features']:assert f['id'] not in seen;seen.add(f['id']);count+=1
   url=next((l['href'] for l in data['links'] if l['rel']=='next'),None)
  assert count==fixture['expected_features'];report['large_file']={'features':count,'pages':pages,'seconds':round(time.monotonic()-started,3)}
 assert not report['page_errors'],report['page_errors'];report['success']=True;Path(a.report).write_text(json.dumps(report,ensure_ascii=False,indent=2));print(json.dumps(report,ensure_ascii=False,indent=2));browser.close()
