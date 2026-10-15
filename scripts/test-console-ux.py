#!/usr/bin/env python3
"""Verify focus, unsaved edits, empty states and responsive keys in a disposable service."""
import argparse
import json
import uuid
from pathlib import Path
from playwright.sync_api import sync_playwright, expect, Error as PlaywrightError
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', required=True)
parser.add_argument('--api-key-file', required=True)
parser.add_argument('--member-id', required=True, help='Existing user without API Keys in the disposable fixture')
parser.add_argument('--chromium')
parser.add_argument('--axe', type=Path, help='Optional local axe-core/axe.min.js for full-page accessibility checks')
parser.add_argument('--output', type=Path, default=Path('target/console-ux'))
args = parser.parse_args()
base = args.url.rstrip('/')
report = {'focus': [], 'dirty': {}, 'mobile': [], 'axe': [], 'errors': []}
shots = args.output
shots.mkdir(parents=True, exist_ok=True)
with sync_playwright() as p:
    b = p.chromium.launch(headless=True, executable_path=args.chromium)
    context = b.new_context(viewport={'width': 1440, 'height': 1000})
    r = context.request.post(base + '/api/session', data={'api_key': Path(args.api_key_file).read_text().strip()}, headers={'Origin': base})
    assert r.ok
    page = context.new_page()
    page.on('pageerror', lambda e: report['errors'].append(str(e)))

    def goto(route):
        page.goto(base + route)
        page.wait_for_load_state('networkidle')

    def call(body):
        r = context.request.post(base + '/api/console', data=body, headers={'Origin': base})
        assert r.ok, (body['action'], r.status)
        return r.json()
    goto('/projects')
    project = call({'action': 'createProject', 'name': 'UX ' + uuid.uuid4().hex[:8]})['id']
    for width in [1440, 390]:
        page.set_viewport_size({'width': width, 'height': 1000})
        for route, label in [('projects', '创建项目'), ('datasets', '创建数据集'), ('users', '新增用户'), ('api-keys', '创建 API Key'), ('access', '设置成员权限')]:
            goto('/' + route + ('?project=' + project if route in ['datasets', 'access'] else ''))
            trigger = page.get_by_role('button', name=label, exact=True)
            trigger.click()
            dialog = page.get_by_role('dialog')
            expect(dialog).to_be_visible()
            dialog.press('Escape')
            expect(dialog).to_have_count(0)
            expect(trigger).to_be_focused()
            report['focus'].append({'width': width, 'dialog': label, 'restored': True})
    page.set_viewport_size({'width': 1440, 'height': 1000})
    goto('/profile')
    display = page.get_by_label('显示名称', exact=True)
    original = display.input_value()
    display.fill('unsaved-profile-marker')
    target = page.get_by_role('link', name='项目', exact=True)
    target.click()
    alert = page.get_by_role('alertdialog', name='放弃未保存的修改？')
    expect(alert).to_be_visible()
    alert.get_by_role('button', name='取消', exact=True).click()
    expect(display).to_have_value('unsaved-profile-marker')
    assert '/profile' in page.url
    target.click()
    alert.get_by_role('button', name='放弃修改', exact=True).click()
    page.wait_for_url('**/projects*')
    goto('/profile')
    expect(display).to_have_value(original)
    report['dirty']['profile_cancel_and_discard'] = True
    goto('/datasets?project=' + project)
    page.get_by_role('button', name='创建数据集', exact=True).click()
    page.get_by_label('名称', exact=True).fill('UX points')
    page.get_by_role('button', name='创建', exact=True).click()
    expect(page.get_by_role('dialog')).to_have_count(0)
    page.get_by_role('button', name='UX points', exact=True).click()
    page.get_by_role('button', name='新建工作区', exact=True).click()
    expect(page.get_by_role('heading', level=1)).to_be_attached()
    trigger = page.get_by_role('button', name='添加要素', exact=True)
    trigger.click()
    dialog = page.get_by_role('dialog')
    identity = page.get_by_label('要素标识', exact=True)
    editor = page.get_by_label('GeoJSON', exact=True)
    identity.fill('ux-one')
    raw = json.loads(editor.input_value())
    raw['properties']['unsaved_test_marker'] = 'retained'
    text = json.dumps(raw)
    editor.fill(text)
    seen_unload = []

    def cancel_reload(prompt):
        seen_unload.append(prompt.type)
        prompt.dismiss()
    page.once('dialog', cancel_reload)
    try:
        page.reload(wait_until='commit', timeout=3000)
    except PlaywrightError:
        pass
    assert seen_unload == ['beforeunload'], seen_unload
    expect(editor).to_have_value(text)
    report['dirty']['reload_cancel_retains_text'] = True
    for dismiss in ['overlay', 'escape', 'cancel', 'close']:
        if dismiss == 'overlay':
            page.locator('[data-slot=dialog-overlay], [data-slot=sheet-overlay]').click(position={'x': 5, 'y': 5})
        elif dismiss == 'escape':
            dialog.press('Escape')
        elif dismiss == 'cancel':
            dialog.get_by_role('button', name='取消', exact=True).click()
        else:
            dialog.get_by_role('button', name='Close', exact=True).click()
        alert = page.get_by_role('alertdialog', name='放弃未保存的修改？')
        expect(alert).to_be_visible()
        page.screenshot(path=str(shots / ('dirty-' + dismiss + '.png')))
        alert.get_by_role('button', name='取消', exact=True).click()
        expect(editor).to_have_value(text)
        expect(identity).to_have_value('ux-one')
        report['dirty'][dismiss + '_cancel_retains_text'] = True
    dialog.get_by_role('button', name='保存到工作区', exact=True).click()
    expect(dialog).to_have_count(0)
    expect(page.get_by_role('alertdialog')).to_have_count(0)
    expect(page.locator('.feature-row', has_text='ux-one')).to_be_visible()
    page.get_by_role('button', name='编辑', exact=True).click()
    assert 'unsaved_test_marker' in editor.input_value()
    editor.fill(editor.input_value().replace('retained', 'discard-me'))
    dialog.press('Escape')
    page.get_by_role('alertdialog').get_by_role('button', name='放弃修改', exact=True).click()
    expect(dialog).to_have_count(0)
    page.get_by_role('button', name='编辑', exact=True).click()
    assert 'discard-me' not in editor.input_value()
    dialog.press('Escape')
    expect(dialog).to_have_count(0)
    report['dirty']['save_and_discard'] = True
    axe = args.axe.read_text() if args.axe else None
    if axe:
        page.evaluate(axe)
    v = page.evaluate('async()=> (await axe.run()).violations.map(x=>({id:x.id,impact:x.impact,nodes:x.nodes.map(n=>({target:n.target,summary:n.failureSummary,html:n.html}))}))') if axe else []
    assert not v, v
    report['axe'].append({'page': 'map', 'checked': bool(axe), 'violations': v})
    goto('/users')
    page.get_by_role('radiogroup', name='用户状态').get_by_role('radio', name='禁用', exact=True).click()
    expect(page.get_by_text('没有匹配的用户', exact=True)).to_be_visible()
    report['empty_users'] = True
    goto('/api-keys')
    page.get_by_label('用户 ID', exact=True).fill(args.member_id)
    page.get_by_role('button', name='查看用户密钥').click()
    expect(page.get_by_text('暂无 API Key', exact=True)).to_be_visible()
    report['empty_keys'] = True
    for width in [320, 390, 768, 1440]:
        page.set_viewport_size({'width': width, 'height': 1000})
        goto('/api-keys')
        expect(page.locator('.account-keys-table tbody tr').first).to_be_visible()
        page.screenshot(path=str(shots / ('keys-' + str(width) + '.png')), full_page=True)
        m = page.evaluate("()=>{let e=document.querySelector('.account-keys-table [data-slot=table-container]');return {document:document.documentElement.scrollWidth,width:innerWidth,client:e.clientWidth,scroll:e.scrollWidth,label:document.querySelector('#key-owner').getBoundingClientRect().x,title:document.querySelector('[data-slot=card-title]').getBoundingClientRect().x}}")
        if width <= 1024:
            cells = page.locator('.account-keys-table tbody tr').first.locator('td').evaluate_all('cells => cells.map(e => ({top:e.getBoundingClientRect().top,bottom:e.getBoundingClientRect().bottom}))')
            assert all(a['bottom'] <= b['top'] for a,b in zip(cells,cells[1:])), cells
        assert m['document'] <= width, m
        assert m['scroll'] <= m['client'] + 1, m
        assert abs(m['label'] - m['title']) < 1, m
        report['mobile'].append(m)
        if axe:
            page.evaluate(axe)
        v = page.evaluate('async()=> (await axe.run()).violations.map(x=>({id:x.id,impact:x.impact,nodes:x.nodes.map(n=>({target:n.target,summary:n.failureSummary,html:n.html}))}))') if axe else []
        assert not v, v
        report['axe'].append({'page': 'keys', 'width': width, 'checked': bool(axe), 'violations': v})
    assert not report['errors'], report['errors']
    b.close()
(shots / 'result.json').write_text(json.dumps(report, indent=2, ensure_ascii=False))
print(json.dumps(report, ensure_ascii=False))
