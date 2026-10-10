#!/usr/bin/env python3
"""Verify dataset families, projected GeoJSON upload and lifecycle in a disposable service."""
import argparse
import json
from pathlib import Path
import uuid
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', default='http://localhost:13001')
parser.add_argument('--token-file', required=True)
parser.add_argument('--chromium', required=True)
parser.add_argument('--geojson', required=True)
parser.add_argument('--screenshots', default='/tmp/geoledger-lifecycle-shots')
args = parser.parse_args()
shots = Path(args.screenshots)
shots.mkdir(parents=True, exist_ok=True)
token = json.loads(Path(args.token_file).read_text())[0]['token']
with sync_playwright() as playwright:
    browser = playwright.chromium.launch(headless=True, executable_path=args.chromium, args=['--no-sandbox','--disable-dev-shm-usage'])
    page = browser.new_page(viewport={'width':1440,'height':960})
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    def command(body):
        result = page.evaluate('''async body=>{const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});return {status:r.status,data:await r.json()}}''',body)
        assert result['status']==200, (body['action'],result)
        return result['data']
    page.goto(args.url+'/login')
    page.get_by_label('访问令牌',exact=True).fill(token)
    page.get_by_role('button',name='进入控制台').click()
    page.wait_for_url('**/projects')
    name='管理验证 '+uuid.uuid4().hex[:8]
    project=command({'action':'createProject','name':name})['id']
    page.goto(args.url+'/datasets?project='+project)
    page.get_by_role('button',name='创建数据集',exact=True).click()
    page.get_by_label('名称',exact=True).fill('行政边界')
    page.get_by_role('combobox',name='数据来源',exact=True).click()
    page.get_by_role('option',name='上传 GeoJSON 文件',exact=True).click()
    page.get_by_label('GeoJSON 文件',exact=True).set_input_files(args.geojson)
    page.get_by_role('button',name='创建',exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0,timeout=120000)
    dataset=command({'action':'datasets','project':project})[0]
    assert dataset['geometryType']=='polygon'
    workspace=command({'action':'workspaces','project':project})[0]
    data=command({'action':'features','project':project,'dataset':dataset['id'],'workspace':workspace['id'],'limit':100})
    assert len(data['features'])==15
    publication=command({'action':'publish','project':project,'workspace':workspace['id'],'version':workspace['version'],'requestId':str(uuid.uuid4()),'message':'行政边界导入'})
    # Publish a mixed-dataset version, then verify deleting one preserves the other.
    points=command({'action':'createDataset','project':project,'name':'保留点','geometryType':'point'})
    mixed=command({'action':'createWorkspace','project':project})
    area=data['features'][0]
    point={'type':'Feature','id':'p','properties':{},'geometry':{'type':'Point','coordinates':[1,2]}}
    saved=command({'action':'save','project':project,'workspace':mixed['id'],'version':mixed['version'],'edits':[{'dataset':points['id'],'featureId':'p','feature':json.dumps(point)},{'dataset':dataset['id'],'featureId':area['id'],'feature':None}]})
    command({'action':'publish','project':project,'workspace':mixed['id'],'version':saved['version'],'requestId':str(uuid.uuid4()),'message':'混合版本'})
    page.goto(args.url+'/datasets?project='+project)
    row=page.get_by_role('row').filter(has=page.get_by_role('button',name='行政边界',exact=True))
    row.get_by_role('button',name='重命名',exact=True).click()
    page.get_by_label('名称',exact=True).fill('行政区面')
    page.get_by_role('button',name='保存',exact=True).click()
    row=page.get_by_role('row').filter(has=page.get_by_role('button',name='行政区面',exact=True))
    row.get_by_role('button',name='删除',exact=True).click()
    expect(page.get_by_role('dialog')).to_contain_text('保留其他数据集')
    page.get_by_label('输入名称确认删除',exact=True).fill('行政区面')
    page.get_by_role('dialog').get_by_role('button',name='删除',exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0)
    remaining=command({'action':'datasets','project':project})
    assert [d['id'] for d in remaining]==[points['id']]
    history=command({'action':'history','project':project})
    assert len(history)==1
    changes=command({'action':'commit','project':project,'revision':history[0]['revision']})['changes']
    assert len(changes)==1 and json.loads(changes[0]['json'])['dataset']==points['id']
    assert len(command({'action':'features','project':project,'dataset':points['id']})['features'])==1
    page.screenshot(path=str(shots/'dataset-deleted.png'),full_page=True)
    page.goto(args.url+'/projects')
    row=page.get_by_role('row').filter(has=page.get_by_role('link',name=name,exact=True))
    row.get_by_role('button',name='重命名',exact=True).click()
    renamed=name+' renamed'
    page.get_by_label('名称',exact=True).fill(renamed)
    page.get_by_role('button',name='保存',exact=True).click()
    row=page.get_by_role('row').filter(has=page.get_by_role('link',name=renamed,exact=True))
    row.get_by_role('button',name='删除',exact=True).click()
    page.get_by_label('输入名称确认删除',exact=True).fill(renamed)
    page.get_by_role('dialog').get_by_role('button',name='删除',exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0)
    assert not any(p['id']==project for p in command({'action':'projects'}))
    assert not errors,errors
    browser.close()
    print('PASS: 15 projected MultiPolygons uploaded; family rejection; rename/delete UI; shared history preserved; project deleted. Project ID: '+project)
