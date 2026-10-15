#!/usr/bin/env python3
"""Check shadcn Select, Tabs, Popover and conflict buttons in a disposable service."""
import argparse, json, uuid
from pathlib import Path
from playwright.sync_api import sync_playwright, expect
from browser_test_support import add_browser_arguments, launch_browser
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', default='http://localhost:13001')
parser.add_argument('--api-key-file',required=True)
add_browser_arguments(parser)
parser.add_argument('--password-file',required=True)
parser.add_argument('--member-id',required=True,help='Existing stable user UUID in the disposable fixture')
parser.add_argument('--output',default='target/auth-refactor-shadcn')
args=parser.parse_args()
api_key=Path(args.api_key_file).read_text().strip()
password=Path(args.password_file).read_text().strip()
output=Path(args.output);output.mkdir(parents=True,exist_ok=True)
with sync_playwright() as p:
    browser=launch_browser(p, args)
    page=browser.new_page(viewport={'width':1440,'height':1000})
    errors=[]
    page.on('pageerror',lambda error:errors.append(str(error)))
    responses=[]
    def observe_response(response):
        if '/api/' not in response.url:return
        try:action=response.request.post_data_json.get('action')
        except Exception:action=None
        responses.append({'path':response.url.split('/api/',1)[1].split('?',1)[0],'action':action,'status':response.status})
        del responses[:-20]
    page.on('response',observe_response)
    page.goto(args.url+'/login')
    page.get_by_role('tab', name='API Key 登录', exact=True).click()
    page.get_by_label('API Key',exact=True).fill(api_key)
    page.get_by_role('button',name='进入控制台').click()
    page.wait_for_url('**/projects')
    def call(body):
        r=page.evaluate('''async body=>{const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});return {status:r.status,data:await r.json()}}''',body)
        assert r['status']==200,(body['action'],r)
        return r['data']
    project_name='shadcn '+uuid.uuid4().hex[:8]
    project=call({'action':'createProject','name':project_name})['id']
    page.goto(args.url+'/access?project='+project)
    expect(page.get_by_label('成员身份',exact=True)).to_have_count(0)
    trigger=page.get_by_role('button',name='设置成员权限',exact=True)
    trigger.click()
    member_dialog=page.get_by_role('dialog',name='设置成员权限',exact=True)
    expect(member_dialog).to_be_visible()
    member_dialog.get_by_label('成员身份',exact=True).fill(args.member_id)
    member_dialog.get_by_role('combobox',name='项目角色',exact=True).click()
    page.get_by_role('option',name='编辑者 · 编辑与发布',exact=True).click()
    def reject_member(route):
        import json
        body=json.loads(route.request.post_data or '{}')
        if body.get('action')=='setMember':
            route.fulfill(status=403,content_type='application/json',body=json.dumps({'error':'测试权限拒绝'}))
        else:route.continue_()
    page.route('**/api/console',reject_member)
    member_dialog.get_by_label('当前密码',exact=True).fill(password)
    member_dialog.get_by_role('button',name='保存权限',exact=True).click()
    expect(member_dialog.get_by_role('alert')).to_be_visible()
    expect(member_dialog.get_by_label('成员身份',exact=True)).to_have_value(args.member_id)
    page.unroute('**/api/console',reject_member)
    member_dialog.get_by_role('button',name='保存权限',exact=True).click()
    expect(member_dialog).to_have_count(0)
    expect(page.get_by_role('row').filter(has_text=args.member_id)).to_contain_text('编辑者')
    trigger.click();page.keyboard.press('Escape');expect(member_dialog).to_have_count(0)
    expect(trigger).to_be_focused()
    expect(page.get_by_role('button',name='归档项目',exact=True)).to_have_count(0)
    page.goto(args.url+'/datasets?project='+project)
    page.get_by_role('button',name='创建数据集',exact=True).click()
    dialog=page.get_by_role('dialog')
    expect(dialog).to_be_visible()
    assert dialog.evaluate("e => getComputedStyle(e).animationName") != 'none', 'Dialog animation CSS missing'
    page.emulate_media(reduced_motion='reduce')
    assert dialog.evaluate("e => getComputedStyle(e).animationName") == 'none', 'Reduced motion ignored'
    page.emulate_media(reduced_motion='no-preference')
    page.get_by_label('名称',exact=True).fill('控件验证')
    select=page.get_by_role('combobox',name='几何类型',exact=True)
    expect(select).to_have_attribute('data-slot','select-trigger')
    select.focus()
    select.press('Space')
    expect(page.get_by_role('listbox')).to_be_visible()
    page.get_by_role('option',name='面（Polygon / MultiPolygon）',exact=True).click()
    expect(select).to_contain_text('面')
    select.press('Space')
    page.get_by_role('option',name='点（Point / MultiPoint）',exact=True).click()
    page.get_by_role('button',name='创建',exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0)
    dataset=call({'action':'datasets','project':project})[0]['id']
    def workspace():return call({'action':'createWorkspace','project':project})['id']
    def save(w,value):
        return call({'action':'save','project':project,'workspace':w,'version':'0','edits':[{'dataset':dataset,'featureId':k,'feature':json.dumps({'type':'Feature','id':k,'properties':{'value':value},'geometry':None})} for k in ['one','two']]})['version']
    def publish(w,v):return call({'action':'publish','project':project,'workspace':w,'version':v,'requestId':str(uuid.uuid4()),'message':'组件验证'})
    seed=workspace();publish(seed,save(seed,0))
    left=workspace();right=workspace();lv=save(left,1);save(right,2);publish(left,lv)
    page.goto(args.url+'/workspaces?project='+project)
    graph=page.get_by_role('tab',name='版本图',exact=True)
    table=page.get_by_role('tab',name='表格',exact=True)
    expect(graph).to_have_attribute('data-slot','tabs-trigger')
    graph.focus();graph.press('ArrowRight')
    expect(table).to_be_focused();expect(table).to_have_attribute('aria-selected','true')
    expect(page.get_by_role('tabpanel',name='表格',exact=True)).to_be_visible()
    table.press('ArrowLeft')
    expect(graph).to_have_attribute('aria-selected','true')
    source=page.get_by_role('button',name='查看版本 r1 的来源',exact=True)
    row=page.get_by_role('button',name='查看版本 r1 的变更',exact=True)
    centers=row.evaluate("e => [...e.parentElement.children].map(c => {const r=c.getBoundingClientRect();return r.y+r.height/2})")
    assert max(centers)-min(centers)<1, centers
    source.click()
    expect(page.locator('[data-slot="popover-content"]')).to_contain_text(seed)
    page.keyboard.press('Escape');expect(source).to_be_focused()
    page.get_by_role('button',name='更多操作 '+right,exact=True).click()
    page.get_by_role('menuitem',name='解决冲突',exact=True).click()
    conflict=page.get_by_role('navigation',name='冲突要素',exact=True)
    expect(conflict.get_by_role('button')).to_have_count(2)
    for button in conflict.get_by_role('button').all():expect(button).to_have_attribute('data-slot','button')
    progress=page.get_by_role('progressbar',name='解决进度')
    expect(progress).to_have_attribute('aria-valuenow','0')
    second=conflict.get_by_role('button').nth(1);second.click();expect(second).to_have_attribute('aria-current','true')
    page.screenshot(path=str(output/'conflict.png'),full_page=True)
    page.get_by_role('dialog').get_by_role('button',name='取消',exact=True).click()
    page.set_viewport_size({'width':390,'height':844})
    table.click();expect(page.get_by_role('tabpanel',name='表格',exact=True)).to_be_visible()
    page.screenshot(path=str(output/'mobile.png'),full_page=True)
    page.goto(args.url+'/access?project='+project)
    page.get_by_role('button',name='设置成员权限',exact=True).click()
    expect(page.get_by_role('dialog',name='设置成员权限',exact=True)).to_be_visible()
    page.get_by_role('button',name='取消',exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0)
    page.goto(args.url+'/api-keys')
    page.get_by_role('button',name='创建 API Key',exact=True).click()
    page.get_by_label('名称',exact=True).fill('release-browser')
    page.get_by_label('当前密码',exact=True).fill(password)
    page.wait_for_load_state('networkidle')
    expect(page.get_by_role('dialog')).to_have_count(1)
    expect(page.get_by_label('名称',exact=True)).to_have_value('release-browser')
    assert page.url == args.url + '/api-keys', 'Global account form must not remount during project restoration'
    try:
        page.get_by_role('button',name='确认创建 API Key',exact=True).click()
    except Exception:
        print('Key dialog diagnostic:',json.dumps({'path':page.url.split(args.url)[-1],
              'dialogs':page.get_by_role('dialog').all_text_contents(),'responses':responses},ensure_ascii=False))
        page.screenshot(path=str(output/'key-dialog-failure.png'),full_page=True,mask=[page.locator('input')])
        raise
    issued=page.get_by_label('新密钥',exact=True)
    expect(issued).to_be_visible()
    issued_key=issued.input_value()
    page.get_by_role('button',name='我已安全保存',exact=True).click()
    member_context=browser.new_context()
    response=member_context.request.post(args.url+'/api/session',data={'api_key':issued_key},headers={'Origin':args.url})
    assert response.status==200,response.status
    row=page.get_by_role('row').filter(has_text='release-browser')
    row.get_by_role('button',name='撤销',exact=True).click()
    page.get_by_label('当前密码',exact=True).fill(password)
    page.get_by_role('button',name='确认撤销 API Key',exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0)
    response=member_context.request.post(args.url+'/api/console',data={'action':'projects'},headers={'Origin':args.url})
    assert response.status==401,response.status
    member_context.close()
    page.goto(args.url+'/project-status?project='+project)
    expect(page.get_by_role('heading',name='项目状态',exact=True)).to_be_visible()
    page.get_by_role('button',name='归档项目',exact=True).click()
    page.get_by_role('button',name='恢复项目',exact=True).click()
    expect(page.get_by_role('button',name='归档项目',exact=True)).to_be_enabled()
    page.get_by_role('button',name='删除项目',exact=True).click()
    delete_dialog=page.get_by_role('dialog',name='删除项目',exact=True)
    confirmation=delete_dialog.get_by_label('删除项目：输入项目名称',exact=False)
    confirmation.fill('wrong-name')
    delete_dialog.get_by_role('button',name='删除项目',exact=True).click()
    expect(page.get_by_role('alert')).to_be_visible()
    confirmation.fill(project_name)
    delete_dialog.get_by_role('button',name='删除项目',exact=True).click()
    page.wait_for_url('**/projects')
    assert not errors,errors
    print('PASS: Select keyboard/form submission, Tabs keyboard/panels, Popover focus, conflict Buttons, mobile view, member dialog save/error/cancel/focus, independent project status and confirmed deletion; no page errors')
    browser.close()
