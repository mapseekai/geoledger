#!/usr/bin/env python3
"""Run against an isolated PostGIS service with business.roads(id,name,geom)."""
import argparse,json,uuid,re
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--url',required=True);p.add_argument('--token-file',required=True);p.add_argument('--chromium',required=True);p.add_argument('--screenshots',default='/tmp/geoledger-postgis-shots');a=p.parse_args()
token=json.loads(Path(a.token_file).read_text())[0]['token']
with sync_playwright() as pw:
    browser=pw.chromium.launch(headless=True,executable_path=a.chromium)
    context=browser.new_context(viewport={'width':1440,'height':960});page=context.new_page();errors=[]
    page.on('pageerror',lambda error:errors.append(str(error)))
    page.goto(a.url+'/login');page.get_by_label('访问令牌',exact=True).fill(token);page.get_by_role('button',name='进入控制台').click();page.wait_for_url('**/projects')
    def call(body):
        response=context.request.post(a.url+'/api/console',data=body,headers={'Origin':a.url})
        assert response.status==200,response.text()
        return json.loads(response.text())
    project=call({'action':'createProject','name':'PostGIS '+uuid.uuid4().hex[:8]})['id']
    page.goto(a.url+'/datasets?project='+project)
    page.get_by_role('button',name='创建数据集',exact=True).click();page.get_by_label('名称',exact=True).fill('已有道路')
    page.get_by_role('combobox',name='数据来源',exact=True).click();page.get_by_role('option',name='接入 PostGIS 已有表',exact=True).click()
    page.get_by_label('Schema',exact=True).fill('business');page.get_by_label('数据表',exact=True).fill('roads')
    page.get_by_role('button',name='创建',exact=True).click();expect(page.get_by_role('dialog')).to_have_count(0,timeout=30000)
    dataset=call({'action':'datasets','project':project})[0]
    assert dataset['geometryType']=='line' and dataset['postgisTable']['table']=='roads'
    assert call({'action':'project','project':project})['head']=='1'
    expect(page.locator('.map-canvas canvas')).to_be_visible(timeout=30000)
    expect(page.locator('.feature-row')).to_have_count(1,timeout=30000)
    page.get_by_role('button',name='新建工作区',exact=True).click()
    page.locator('.feature-row').first.click();page.get_by_role('button',name='编辑',exact=True).click()
    feature={'type':'Feature','properties':{'name':'Edited in GeoLedger'},'geometry':{'type':'LineString','coordinates':[[104,35],[106,37]]}}
    page.get_by_label('GeoJSON',exact=True).fill(json.dumps(feature))
    page.get_by_role('button',name='保存到工作区',exact=True).click();expect(page.get_by_role('dialog')).to_have_count(0,timeout=30000)
    page.get_by_role('button',name='发布版本',exact=True).click()
    page.get_by_label('版本说明',exact=True).fill('Web table binding')
    page.get_by_role('button',name='确认发布',exact=True).click()
    expect(page.get_by_role('heading',name='版本 r2 已发布',exact=True)).to_be_visible(timeout=30000)
    page.get_by_role('button',name='完成',exact=True).click()
    expect(page.locator('.map-progress')).to_have_count(0,timeout=30000)
    expect(page.get_by_role('alert').filter(has_text=re.compile(r'\S'))).to_have_count(0)
    result=call({'action':'features','project':project,'dataset':dataset['id']})
    assert result['features'][0]['properties']['name']=='Edited in GeoLedger'
    Path(a.screenshots).mkdir(parents=True,exist_ok=True);page.screenshot(path=str(Path(a.screenshots)/'postgis-bound-table.png'),full_page=True)
    assert not errors,errors
    browser.close()
print('PostGIS browser: existing table attachment, inferred schema, initial snapshot, edit and publication passed')
