#!/usr/bin/env python3
"""Verify per-dataset workspace and history counts against an isolated service."""
import argparse
import json
from pathlib import Path
import uuid
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', default='http://localhost:3000')
parser.add_argument('--token-file', required=True)
parser.add_argument('--chromium')
parser.add_argument('--screenshots', default='/tmp/geoledger-change-shots')
args = parser.parse_args()
shots = Path(args.screenshots)
shots.mkdir(parents=True, exist_ok=True)
token = json.loads(Path(args.token_file).read_text())[0]['token']
with sync_playwright() as p:
    browser = p.chromium.launch(headless=True, executable_path=args.chromium,
                               args=['--no-sandbox', '--disable-dev-shm-usage'])
    page = browser.new_page(viewport={'width':1600,'height':1000})
    errors=[]
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.goto(args.url+'/login')
    page.get_by_label('访问令牌',exact=True).fill(token)
    page.get_by_role('button',name='进入控制台').click()
    page.wait_for_url('**/projects')
    def command(body):
        result = page.evaluate('''async body => {
          const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});
          return {status:r.status,data:await r.json()};
        }''',body)
        assert result['status']==200,(body,result)
        return result['data']
    project=command({'action':'createProject','name':'变更统计 '+uuid.uuid4().hex[:8]})['id']
    roads=command({'action':'createDataset','geometryType':'point','project':project,'name':'道路'})['id']
    buildings=command({'action':'createDataset','geometryType':'point','project':project,'name':'建筑'})['id']
    def edit(dataset, key, value):
        return {'dataset':dataset,'featureId':key,'feature':None if value is None else json.dumps({'type':'Feature','id':key,'properties':{'value':value},'geometry':None})}
    def save(workspace, edits):
        return command({'action':'save','project':project,'workspace':workspace,'version':'0','edits':edits})['version']
    def publish(workspace, version):
        return command({'action':'publish','project':project,'workspace':workspace,'version':version,'requestId':str(uuid.uuid4()),'message':'按数据集记录变更'})
    seed=command({'action':'createWorkspace','project':project})['id']
    publish(seed,save(seed,[edit(dataset,key,0) for dataset in [roads,buildings] for key in ['modify','delete']]))
    workspace=command({'action':'createWorkspace','project':project})['id']
    edits=[edit(roads,f'added-{i:02d}',1) for i in range(25)]
    edits.extend([edit(roads,'modify',1),edit(roads,'delete',None),edit(buildings,'added',1),edit(buildings,'modify',1),edit(buildings,'delete',None)])
    version=save(workspace,edits)
    summary=command({'action':'workspaceSummary','project':project,'workspace':workspace})
    assert summary['total']=={'added':26,'deleted':2,'modified':2},summary
    by_name={row['name']:row for row in summary['datasets']}
    assert [by_name['道路'][key] for key in ['added','deleted','modified']]==[25,1,1]
    assert [by_name['建筑'][key] for key in ['added','deleted','modified']]==[1,1,1]
    page.goto(args.url+'/workspaces?project='+project)
    expect(page.get_by_label('版本图表头',exact=True)).to_be_visible()
    expect(page.get_by_label('版本图表头',exact=True)).to_contain_text('数据集 / 要素变更')
    expect(page.get_by_role('tabpanel',name='版本图',exact=True)).to_contain_text('道路：新增 25 · 删除 1 · 修改 1')
    page.screenshot(path=str(shots/'workspace-graph.png'),full_page=True)
    page.get_by_role('tab',name='表格',exact=True).click()
    table=page.get_by_role('table',name='工作区列表',exact=True)
    expect(table).to_contain_text('工作区列表与数据集变更')
    row=table.get_by_role('row').filter(has_text='道路：新增 25')
    expect(row).to_contain_text('建筑：新增 1 · 删除 1 · 修改 1')
    row.get_by_role('button',name='变更',exact=True).click()
    summary_table=page.get_by_role('table',name='按数据集统计要素变更',exact=True)
    expect(summary_table.get_by_role('row').filter(has_text='道路')).to_have_text('道路2511')
    expect(summary_table.get_by_role('row').filter(has_text='建筑')).to_have_text('建筑111')
    expect(summary_table.get_by_role('row').filter(has_text='合计')).to_have_text('合计2622')
    page.screenshot(path=str(shots/'workspace-change-summary.png'),full_page=True)
    result=publish(workspace,version)
    historical=command({'action':'commitSummary','project':project,'revision':result['revision']})
    assert historical==summary,(historical,summary)
    page.goto(args.url+'/history?project='+project)
    table=page.get_by_role('table',name='版本历史',exact=True)
    row=table.get_by_role('row').filter(has_text='r'+result['revision'])
    expect(row).to_contain_text('道路：新增 25 · 删除 1 · 修改 1')
    expect(row).to_contain_text('建筑：新增 1 · 删除 1 · 修改 1')
    page.screenshot(path=str(shots/'history-change-summary.png'),full_page=True)
    row.get_by_role('button',name='详情',exact=True).click()
    expect(page.get_by_role('table',name='按数据集统计要素变更').get_by_role('row').filter(has_text='合计')).to_have_text('合计2622')
    page.set_viewport_size({'width':390,'height':844})
    expect(page.get_by_role('table',name='按数据集统计要素变更')).to_be_visible()
    page.screenshot(path=str(shots/'history-change-summary-mobile.png'),full_page=True)
    assert not errors,errors
    browser.close()
    print('Per-dataset summaries: multiple pages, adds/deletes/modifications, graph headers, workspace table and version history passed')
