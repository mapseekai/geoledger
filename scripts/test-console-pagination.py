#!/usr/bin/env python3
"""Verify diff/commit/history/audit page endings through an isolated production console."""
import argparse
import json
from pathlib import Path
import uuid
from playwright.sync_api import sync_playwright, expect
from browser_test_support import add_browser_arguments, launch_browser


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', default='http://localhost:3000')
    parser.add_argument('--api-key-file', required=True)
    parser.add_argument('--screenshots', default='/tmp/geoledger-pagination-shots')
    add_browser_arguments(parser)
    args = parser.parse_args()
    shots = Path(args.screenshots)
    shots.mkdir(parents=True, exist_ok=True)
    api_key = Path(args.api_key_file).read_text().strip()
    with sync_playwright() as playwright:
        browser = launch_browser(playwright, args)
        page = browser.new_page(viewport={'width': 1600, 'height': 1000})
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        page.goto(args.url + '/login')
        page.get_by_role('tab', name='API Key 登录', exact=True).click()
        page.get_by_label('API Key', exact=True).fill(api_key)
        page.get_by_role('button', name='进入控制台', exact=True).click()
        page.wait_for_url('**/projects')

        def command(body):
            response = page.evaluate("""async body => {
              const response = await fetch('/api/console', {method:'POST', headers:{'content-type':'application/json'}, body:JSON.stringify(body)});
              return {status:response.status, data:await response.json()};
            }""", body)
            assert response['status'] == 200, (body['action'], response)
            return response['data']

        project = command({'action': 'createProject', 'name': 'query pages ' + uuid.uuid4().hex[:8]})['id']
        dataset = command({'action': 'createDataset', 'geometryType': 'point', 'project': project, 'name': 'points'})['id']
        page.goto(args.url + '/history?project=' + project)
        expect(page.get_by_text('还没有发布记录', exact=True)).to_be_visible()
        expect(page.get_by_role('button', name='下一页', exact=True)).to_be_disabled()

        def workspace():
            return command({'action': 'createWorkspace', 'project': project})['id']

        def save(draft, keys):
            edits = [{'dataset': dataset, 'featureId': key, 'feature': '{"type":"Feature","id":' + json.dumps(key) + ',"properties":{"exact":9007199254740993},"geometry":null}'} for key in keys]
            return command({'action': 'save', 'project': project, 'workspace': draft, 'version': '0', 'edits': edits})['version']

        def publish(draft, version, message):
            return command({'action': 'publish', 'project': project, 'workspace': draft, 'version': version, 'requestId': str(uuid.uuid4()), 'message': message})

        draft = workspace()
        version = save(draft, [f'key/{number:02}' for number in range(50)])
        terminal = command({'action': 'diff', 'project': project, 'workspace': draft, 'limit': 50})
        assert len(terminal['changes']) == 50 and terminal.get('nextAfter') is None
        page.goto(args.url + '/workspaces?project=' + project)
        page.get_by_role('tab', name='表格', exact=True).click()
        table = page.get_by_role('table', name='工作区列表', exact=True)
        table.get_by_role('button', name='变更', exact=True).click()
        dialog = page.get_by_role('dialog')
        expect(dialog.locator('.filter-count').first).to_have_text('50')
        expect(dialog.get_by_role('button', name='加载更多', exact=True)).to_have_count(0)
        page.screenshot(path=str(shots / 'diff-exact-terminal.png'), full_page=True)
        publish(draft, version, 'fifty changes')
        for number in range(2, 21):
            draft = workspace()
            publish(draft, save(draft, [f'append/{number:02}']), f'revision {number}')

        history = command({'action': 'history', 'project': project, 'limit': 20})
        assert [entry['revision'] for entry in history['commits']] == [str(number) for number in range(1, 21)]
        assert history.get('nextAfter') is None
        page.goto(args.url + '/history?project=' + project)
        table = page.get_by_role('table', name='版本历史', exact=True)
        expect(table.get_by_role('row')).to_have_count(21)
        expect(page.get_by_role('button', name='下一页', exact=True)).to_be_disabled()
        page.screenshot(path=str(shots / 'history-exact-terminal.png'), full_page=True)
        table.get_by_role('row').filter(has=page.get_by_text('r1', exact=True)).get_by_role('button', name='详情', exact=True).click()
        dialog = page.get_by_role('dialog')
        expect(dialog.locator('.filter-count').first).to_have_text('50')
        expect(dialog.get_by_role('button', name='加载更多', exact=True)).to_have_count(0)
        page.screenshot(path=str(shots / 'commit-exact-terminal.png'), full_page=True)

        # Pad with real project events so audit has an exact 20-row final page.
        audit = command({'action': 'audit', 'project': project, 'limit': 100})
        for _ in range((-len(audit['events'])) % 20):
            workspace()
        audit = command({'action': 'audit', 'project': project, 'limit': 100})
        ids = [entry['id'] for entry in audit['events']]
        assert ids and len(ids) % 20 == 0 and audit.get('nextAfter') is None
        page.goto(args.url + '/audit?project=' + project)
        seen = []
        for number in range(len(ids) // 20):
            table = page.get_by_role('table')
            expect(table.get_by_role('row')).to_have_count(21)
            expected = ids[number * 20:(number + 1) * 20]
            expect(table.get_by_role('row').nth(1).get_by_role('cell').first).to_have_text('#' + expected[0])
            seen.extend([text.lstrip('#') for text in table.locator('tbody tr td:first-child').all_text_contents()])
            next_page = page.get_by_role('button', name='下一页', exact=True)
            if number + 1 == len(ids) // 20:
                expect(next_page).to_be_disabled()
            else:
                expect(next_page).to_be_enabled()
                next_page.click()
        assert seen == ids
        page.screenshot(path=str(shots / 'audit-exact-terminal.png'), full_page=True)
        page.set_viewport_size({'width': 390, 'height': 844})
        expect(page.get_by_role('button', name='下一页', exact=True)).to_be_disabled()
        page.screenshot(path=str(shots / 'audit-mobile-terminal.png'), full_page=True)
        assert not errors, errors
        browser.close()
        print(f'Query pagination passed: empty/exact history, 50-row diff/commit, {len(ids)} audit events without duplicates or omissions; desktop/mobile sandboxed Chromium.')


if __name__ == '__main__':
    main()
