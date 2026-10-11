#!/usr/bin/env python3
"""Check shadcn Select, Tabs, Popover and conflict buttons in a disposable service."""
import argparse, json, uuid
from pathlib import Path
from playwright.sync_api import sync_playwright, expect
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', default='http://localhost:13001')
parser.add_argument('--token-file',required=True)
parser.add_argument('--chromium',required=True)
args=parser.parse_args()
token=json.loads(Path(args.token_file).read_text())[0]['token']
with sync_playwright() as p:
    browser=p.chromium.launch(headless=True,executable_path=args.chromium,args=['--no-sandbox','--disable-dev-shm-usage'])
    page=browser.new_page(viewport={'width':1440,'height':1000})
    errors=[]
    page.on('pageerror',lambda error:errors.append(str(error)))
    page.goto(args.url+'/login')
    page.get_by_label('访问令牌',exact=True).fill(token)
    page.get_by_role('button',name='进入控制台').click()
    page.wait_for_url('**/projects')
    def call(body):
        r=page.evaluate('''async body=>{const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});return {status:r.status,data:await r.json()}}''',body)
        assert r['status']==200,(body['action'],r)
        return r['data']
    project=call({'action':'createProject','name':'shadcn '+uuid.uuid4().hex[:8]})['id']
    page.goto(args.url+'/datasets?project='+project)
    page.get_by_role('button',name='创建数据集',exact=True).click()
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
    source.click()
    expect(page.locator('[data-slot="popover-content"]')).to_contain_text(seed)
    page.keyboard.press('Escape');expect(source).to_be_focused()
    page.get_by_role('button',name='更多操作 '+right,exact=True).click()
    page.get_by_role('menuitem',name='解决冲突',exact=True).click()
    conflict=page.get_by_role('navigation',name='冲突要素',exact=True)
    expect(conflict.get_by_role('button')).to_have_count(2)
    for button in conflict.get_by_role('button').all():expect(button).to_have_attribute('data-slot','button')
    second=conflict.get_by_role('button').nth(1);second.click();expect(second).to_have_attribute('aria-current','true')
    page.screenshot(path='/tmp/geoledger-shadcn-conflict.png',full_page=True)
    page.get_by_role('dialog').get_by_role('button',name='取消',exact=True).click()
    page.set_viewport_size({'width':390,'height':844})
    table.click();expect(page.get_by_role('tabpanel',name='表格',exact=True)).to_be_visible()
    page.screenshot(path='/tmp/geoledger-shadcn-mobile.png',full_page=True)
    assert not errors,errors
    print('PASS: Select keyboard/form submission, Tabs keyboard/panels, Popover focus, conflict Buttons, mobile view; no page errors')
    browser.close()
