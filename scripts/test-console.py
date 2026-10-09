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
        response = page.evaluate('''async body => {const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});return {status:r.status,data:await r.json()};}''', body)
        assert response['status'] == 200, (body['action'], response)
        return response['data']
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
    page.get_by_label('要素标识',exact=True).fill('road-1')
    feature='{"type":"Feature","properties":{"exact":18446744073709551615,"geojson":"keep","detail_json":"keep"},"geometry":{"type":"Point","coordinates":[104,35]}}'
    page.get_by_label('GeoJSON',exact=True).fill(feature)
    page.get_by_role('button',name='保存到工作区',exact=True).click()
    # The saved feature is selected in the list and the inspector.
    expect(page.locator('.feature-row.is-selected',has_text='road-1')).to_be_visible()
    page.get_by_role('button',name='编辑',exact=True).click()
    assert '18446744073709551615' in page.get_by_label('GeoJSON',exact=True).input_value()
    page.get_by_role('button',name='取消',exact=True).click()
    expect(page.locator('.map-canvas canvas')).to_be_visible(timeout=30000)
    expect(page.locator('.map-progress')).to_have_count(0,timeout=30000)
    inspector=page.get_by_role('complementary',name='要素信息')
    expect(inspector).to_contain_text('18446744073709551615')
    inspector.get_by_text('GeoJSON',exact=True).click()
    assert '18446744073709551615' in inspector.locator('pre').inner_text()
    page.get_by_role('button',name='放大',exact=True).click()
    page.get_by_role('button',name='属性表',exact=True).click()
    expect(page.get_by_role('region',name='属性表').get_by_role('cell',name='road-1',exact=True)).to_be_visible()
    page.screenshot(path=str(shots/'next-geojson-map.png'),full_page=True)
    # Reopening exercises map cleanup and worker recreation.
    page.get_by_role('button',name='返回数据集',exact=True).click()
    page.get_by_role('button',name='道路',exact=True).click()
    source=page.get_by_role('combobox',name='数据来源',exact=True)
    source.click()
    page.get_by_role('option').nth(1).click()
    expect(page.locator('.map-canvas canvas')).to_be_visible(timeout=30000)
    page.locator('.feature-row',has_text='road-1').click()
    # Homogeneous collections obey the point dataset family.
    collection=json.loads(feature)
    collection['geometry']={'type':'GeometryCollection','geometries':[
        {'type':'MultiPoint','coordinates':[[104,35],[104.01,35.01]]},
        {'type':'Point','coordinates':[103.99,34.99]}]}
    page.get_by_role('button',name='编辑',exact=True).click()
    page.get_by_label('GeoJSON',exact=True).fill(json.dumps(collection))
    page.get_by_role('button',name='保存到工作区',exact=True).click()
    expect(inspector).to_contain_text('GeometryCollection')
    page.get_by_role('button',name='缩放至要素',exact=True).click()
    page.screenshot(path=str(shots/'next-geojson-collection.png'),full_page=True)
    page.set_viewport_size({'width':390,'height':844})
    expect(page.locator('.map-canvas canvas')).to_be_visible()
    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
    page.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(resolve))))')
    page.screenshot(path=str(shots/'next-geojson-mobile.png'),full_page=True)
    page.set_viewport_size({'width':1440,'height':960})
    page.locator('.feature-row',has_text='road-1').click()
    page.get_by_role('button',name='编辑',exact=True).click()
    page.get_by_label('GeoJSON',exact=True).fill(feature)
    page.get_by_role('button',name='保存到工作区',exact=True).click()
    expect(inspector).to_contain_text('Point')
    captured=[]
    def interrupted(route):
        body=route.request.post_data_json
        if body.get('action')=='publish':
            captured.append(body)
            if len(captured)==1:
                response=route.fetch()
                assert response.status==200
                route.abort('failed')
                return
            if len(captured)==2:
                route.fulfill(status=401,content_type='application/json',body=json.dumps({'error':{'message':'expired test session','uncertain':False}}))
                return
        route.continue_()
    page.route('**/api/console',interrupted)
    page.get_by_role('button',name='发布版本',exact=True).click()
    page.get_by_label('版本说明',exact=True).fill('添加道路')
    page.get_by_role('button',name='确认发布',exact=True).click()
    expect(page.get_by_role('button',name='重试原发布',exact=True)).to_be_enabled()
    page.get_by_role('button',name='关闭',exact=True).click()
    expect(page.get_by_role('button',name='添加要素',exact=True)).to_be_disabled()
    # Reconstruct the request after a reload, even though the workspace is already published.
    page.goto(origin+'/workspaces?project='+project)
    page.get_by_role('button',name='重试原发布',exact=True).click()
    page.get_by_role('dialog').get_by_role('button',name='重试原发布',exact=True).click()
    page.wait_for_url('**/login?expired=1')
    assert page.evaluate('sessionStorage.getItem("gl.publication")') is not None
    page.get_by_label('访问令牌',exact=True).fill(token)
    page.get_by_role('button',name='进入控制台').click()
    page.wait_for_url('**/projects')
    page.goto(origin+'/workspaces?project='+project)
    page.get_by_role('button',name='重试原发布',exact=True).click()
    page.get_by_role('dialog').get_by_role('button',name='重试原发布',exact=True).click()
    expect(page.get_by_text('版本 r1 已发布',exact=True)).to_be_visible()
    assert captured[0]==captured[1]==captured[2]
    page.get_by_role('button',name='完成',exact=True).click()
    assert page.evaluate('sessionStorage.getItem("gl.publication")') is None
    assert command({'action':'project','project':project})['head']=='1'
    datasets=command({'action':'datasets','project':project})
    data=command({'action':'features','project':project,'dataset':datasets[0]['id']})
    assert '18446744073709551615' in data['features'][0]['geojson']
    for index in range(21):
        command({'action':'createDataset','geometryType':'point','project':project,'name':f'分页数据集 {index:02}'})
    page.goto(origin+'/datasets?project='+project)
    expect(page.get_by_text('第 1 页 · 本页 20 项',exact=True)).to_be_visible()
    page.get_by_role('button',name='下一页',exact=True).click()
    expect(page.get_by_text('第 2 页 · 本页 2 项',exact=True)).to_be_visible()
    page.get_by_role('button',name='上一页',exact=True).click()
    expect(page.get_by_text('第 1 页 · 本页 20 项',exact=True)).to_be_visible()
    page.goto(origin+'/history?project='+project)
    expect(page.get_by_role('cell',name='添加道路',exact=True)).to_be_visible()
    page.get_by_role('button',name='详情',exact=True).click()
    expect(page.locator('pre').filter(has_text='18446744073709551615')).to_be_visible()
    page.get_by_role('button',name='Close',exact=True).click()
    page.get_by_role('button',name='撤销',exact=True).click()
    page.get_by_role('button',name='创建撤销工作区',exact=True).click()
    expect(page.get_by_text('工作区已创建：',exact=False)).to_be_visible()
    page.goto(origin+'/access?project='+project)
    page.get_by_label('成员身份',exact=True).fill('browser-test-viewer')
    page.get_by_role('button',name='保存权限',exact=True).click()
    expect(page.get_by_text('成员权限已更新。',exact=True)).to_be_visible()
    expect(page.get_by_role('cell',name='browser-test-viewer',exact=True)).to_be_visible()
    page.get_by_role('button',name='移除成员 browser-test-viewer',exact=True).click()
    expect(page.get_by_role('cell',name='browser-test-viewer',exact=True)).to_have_count(0)
    page.get_by_role('button',name='归档项目',exact=True).click()
    expect(page.get_by_role('button',name='恢复项目',exact=True)).to_be_visible()
    page.get_by_role('button',name='恢复项目',exact=True).click()
    expect(page.get_by_role('button',name='归档项目',exact=True)).to_be_visible()
    page.goto(origin+'/audit?project='+project)
    expect(page.get_by_text('第 1 页 · 本页 20 项',exact=True)).to_be_visible()
    page.get_by_role('button',name='下一页',exact=True).click()
    expect(page.get_by_text('set_member',exact=True)).to_be_visible()
    # Interoperability and concurrent paging: workspace beyond first 100, 256-byte ID,
    # stale draft pages, and a published snapshot pinned while another user publishes.
    edge=command({'action':'createProject','name':'分页边界 '+uuid.uuid4().hex[:8]})['id']
    dataset=command({'action':'createDataset','geometryType':'point','project':edge,'name':'边界要素'})['id']
    workspaces=[command({'action':'createWorkspace','project':edge}) for _ in range(101)]
    listed={w['id'] for w in command({'action':'workspaces','project':edge,'limit':100})}
    workspace=next(w for w in workspaces if w['id'] not in listed)
    spaced_id=' '+'x'*254+' '
    def edit_item(identifier, label):
        return {'dataset':dataset,'featureId':identifier,'feature':json.dumps({'type':'Feature','id':identifier,'properties':{'label':label,'exact':18446744073709551615},'geometry':None})}
    saved=command({'action':'save','project':edge,'workspace':workspace['id'],'version':workspace['version'],'edits':[edit_item(spaced_id,'spaced')]+[edit_item(f'f-{i:02}','original') for i in range(21)]})
    page.goto(origin+'/datasets?project='+edge)
    page.get_by_role('button',name='边界要素',exact=True).click()
    # The data source combobox looks up a workspace that is not on the first page.
    source=page.get_by_role('combobox',name='数据来源',exact=True)
    source.click()
    page.get_by_role('combobox',name='工作区标识',exact=True).fill(workspace['id'])
    page.get_by_role('option',name='使用工作区').click()
    expect(source).to_have_attribute('data-value',workspace['id'])
    inspector=page.get_by_role('complementary',name='要素信息')
    page.locator('.feature-row',has_text='x'*254).click()
    page.get_by_role('button',name='编辑',exact=True).click()
    assert page.get_by_label('要素标识',exact=True).input_value()==spaced_id
    page.get_by_role('button',name='保存到工作区',exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0)
    expect(page.locator('.feature-row')).to_have_count(22)
    expect(page.locator('.map-progress')).to_have_count(0)
    expect(page.get_by_role('button',name='添加要素',exact=True)).to_be_enabled()
    # A stale draft view cannot overwrite a newer workspace version.
    current=command({'action':'workspace','project':edge,'workspace':workspace['id']})
    saved=command({'action':'save','project':edge,'workspace':workspace['id'],'version':current['version'],'edits':[edit_item('f-20','late')]})
    page.locator('.feature-row',has_text='f-00').click()
    page.get_by_role('button',name='编辑',exact=True).click()
    page.get_by_role('button',name='保存到工作区',exact=True).click()
    expect(page.get_by_role('dialog').get_by_role('alert')).to_be_visible()
    page.get_by_role('button',name='取消',exact=True).click()
    page.get_by_role('button',name='刷新',exact=True).click()
    page.locator('.feature-row',has_text='f-20').click()
    expect(inspector).to_contain_text('late')
    command({'action':'publish','project':edge,'workspace':workspace['id'],'version':saved['version'],'requestId':str(uuid.uuid4()),'message':'snapshot one'})
    source.click()
    page.get_by_role('option',name='已发布的数据').click()
    expect(source).to_have_attribute('data-value','')
    expect(page.locator('.feature-row')).to_have_count(22)
    next_workspace=command({'action':'createWorkspace','project':edge})
    next_saved=command({'action':'save','project':edge,'workspace':next_workspace['id'],'version':next_workspace['version'],'edits':[edit_item('f-20','second')]})
    command({'action':'publish','project':edge,'workspace':next_workspace['id'],'version':next_saved['version'],'requestId':str(uuid.uuid4()),'message':'snapshot two'})
    # The loaded published view keeps its revision until refreshed.
    page.locator('.feature-row',has_text='f-20').click()
    expect(inspector).to_contain_text('无几何')
    expect(inspector).to_contain_text('late')
    page.get_by_role('button',name='刷新',exact=True).click()
    expect(inspector).to_contain_text('second')
    page.goto(origin+'/projects')
    expect(page.get_by_role('link',name=name,exact=True)).to_be_visible()
    page.screenshot(path=str(shots/'next-projects.png'),full_page=True)
    page.set_viewport_size({'width':390,'height':844})
    expect(page.get_by_role('button',name='打开导航',exact=True)).to_be_visible()
    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
    page.screenshot(path=str(shots/'next-mobile.png'),full_page=True)
    copied=next(c for c in context.cookies() if c['name']=='gl_session')
    page.get_by_role('button',name='打开导航',exact=True).click()
    page.get_by_role('button',name='退出登录',exact=True).click()
    page.wait_for_url('**/login')
    assert not any(c['name']=='gl_session' for c in context.cookies())
    # Logout revokes the server-side session: a copied cookie no longer authenticates.
    replay=browser.new_context()
    replay.add_cookies([copied])
    response=replay.request.post(origin+'/api/console',data={'action':'info'},headers={'Origin':origin})
    assert response.status==401, response.status
    replay.close()
    assert page.evaluate('localStorage.length')==0
    assert page.evaluate('sessionStorage.getItem("gl.publication")') is None
    page.screenshot(path=str(shots/'next-login-mobile.png'),full_page=True)
    response=context.request.post(origin+'/api/console',data={'action':'info'},headers={'Origin':origin})
    assert response.status==401
    assert not errors, errors
    browser.close()
print('Console passed: session/CSRF, exact JSON, interrupted publication + expired-login retry, 101 workspaces, long IDs, concurrent draft/published paging, history/undo, access/audit, logout (server-side revocation) and mobile layout.')
