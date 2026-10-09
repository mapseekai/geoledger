#!/usr/bin/env python3
"""Exercise the separate Next.js console against a disposable GeoLedger service."""
import argparse
import json
from pathlib import Path
import uuid
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', default='http://127.0.0.1:3000')
parser.add_argument('--token-file', required=True)
parser.add_argument('--chromium')
parser.add_argument('--feature-bytes',type=int,default=128*1024)
parser.add_argument('--screenshots', default='artifacts')
args = parser.parse_args()
token = json.loads(Path(args.token_file).read_text())[0]['token']
shots = Path(args.screenshots)
shots.mkdir(parents=True, exist_ok=True)
origin = args.url.rstrip('/')
with sync_playwright() as p:
    browser = p.chromium.launch(headless=True, executable_path=args.chromium,
                                args=['--no-sandbox', '--disable-dev-shm-usage'])
    context = browser.new_context(viewport={'width':1440,'height':960})
    page = context.new_page()
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    def command(body):
        response = page.evaluate('''async body => {const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});return {status:r.status,data:await r.text()};}''', body)
        assert response['status'] == 200, (body['action'], response)
        return json.loads(response['data'])
    page.goto(origin+'/projects')
    page.wait_for_url('**/login')
    page.screenshot(path=str(shots/'next-login.png'), full_page=True)
    page.get_by_label('访问令牌',exact=True).fill('invalid-test-token')
    page.get_by_role('button',name='进入控制台').click()
    expect(page.get_by_role('alert')).to_be_visible()
    page.get_by_label('访问令牌',exact=True).fill(token)
    page.get_by_role('button',name='进入控制台').click()
    page.wait_for_url('**/projects')
    cookies=context.cookies()
    auth=next(c for c in cookies if c['name']=='gl_session')
    assert auth['httpOnly'] and auth['sameSite']=='Strict' and token not in auth['value']
    assert not page.evaluate('document.cookie.includes("gl_session")')
    assert token not in page.content()
    for headers in [{}, {'Origin':'https://attacker.example'}]:
        response=context.request.post(origin+'/api/console',data={'action':'createProject','name':'blocked'},headers=headers)
        assert response.status==403
    page.wait_for_load_state('networkidle')
    name='浏览器验证 '+uuid.uuid4().hex[:8]
    page.get_by_role('button',name='创建项目',exact=True).click()
    page.get_by_label('名称',exact=True).fill(name)
    page.get_by_role('button',name='创建',exact=True).click()
    page.get_by_role('link',name=name,exact=True).click()
    page.wait_for_url('**/datasets?project=*')
    project=page.url.split('project=')[1]
    page.get_by_role('button',name='创建数据集',exact=True).click()
    page.get_by_label('名称',exact=True).fill('道路')
    page.get_by_role('button',name='创建',exact=True).click()
    page.get_by_role('button',name='道路',exact=True).click()
    page.get_by_role('button',name='新建工作区',exact=True).click()
    page.get_by_role('button',name='添加要素',exact=True).click()
    upload=page.get_by_label('上传 GeoJSON 文件',exact=True)
    upload.set_input_files({'name':'invalid.geojson','mimeType':'application/geo+json','buffer':b'{bad'})
    expect(page.get_by_role('alert')).to_contain_text('有效的 JSON')
    features=[{'type':'Feature','id':f'upload-{i:04d}','properties':{'name':f'Imported {i}','padding':'a'*9000},'geometry':{'type':'Point','coordinates':[104+i/1000,35]}} for i in range(130)]
    features[0]['id']=18446744073709551615
    features[0]['properties']['exact']=18446744073709551615
    features[0]['properties']['padding']='a'*args.feature_bytes
    features[1].pop('id')
    features[1]['properties']=None
    features[2]['bbox']=[104,35,105,36]
    raw=json.dumps({'type':'FeatureCollection','features':features}).encode()
    assert len(raw)>1024*1024
    upload.set_input_files({'name':'large.geojson','mimeType':'application/geo+json','buffer':raw})
    expect(page.get_by_role('status')).to_contain_text('130 个要素')
    saves=[]
    failed=[False]
    def intercept(route):
        body=route.request.post_data_json
        if body.get('action')=='save':
            saves.append(body)
            if len(saves)==2:
                failed[0]=True
                route.abort()
                return
        route.continue_()
    page.route('**/api/console',intercept)
    page.get_by_role('button',name='保存到工作区',exact=True).click()
    expect(page.get_by_role('alert')).to_be_visible(timeout=30000)
    expect(page.get_by_role('status').filter(has_text='已保存')).to_contain_text(str(len(saves[0]['edits']))+' 个要素')
    page.get_by_role('button',name='保存到工作区',exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0,timeout=60000)
    assert failed[0]
    assert saves[1]['edits']==saves[2]['edits']
    assert saves[1]['version']==saves[2]['version']
    dataset=command({'action':'datasets','project':project})[0]['id']
    workspace=command({'action':'workspaces','project':project})[0]['id']
    rows=[]
    after=None
    while True:
        body={'action':'features','project':project,'dataset':dataset,'workspace':workspace,'limit':100}
        if after: body['after']=after
        data=command(body)
        rows.extend(data['features'])
        after=data.get('nextAfter')
        if not after: break
    assert len(rows)==130,len(rows)
    exact=next(row for row in rows if row['id']=='18446744073709551615')
    assert exact['type']=='Feature'
    assert exact['properties']['exact']==18446744073709551615
    assert len(exact['properties']['padding'])==args.feature_bytes
    assert command({'action':'info'})['maxFeatureBytes']==0
    current=command({'action':'workspace','project':project,'workspace':workspace})
    command({'action':'publish','project':project,'workspace':workspace,'version':current['version'],'requestId':str(uuid.uuid4()),'message':'large GeoJSON browser validation'})
    published=command({'action':'features','project':project,'dataset':dataset,'featureId':'18446744073709551615'})
    assert len(published['features'][0]['properties']['padding'])==args.feature_bytes
    page.get_by_role('button',name='添加要素',exact=True).click()
    page.set_viewport_size({'width':390,'height':844})
    expect(page.get_by_label('上传 GeoJSON 文件',exact=True)).to_be_visible()
    page.screenshot(path=str(shots/'geojson-upload-mobile.png'),full_page=True)
    assert not errors,errors
    browser.close()
    print('GeoJSON upload: invalid input, >1 MiB, 130 features, interrupted batch resume, exact IDs/properties and mobile passed')
