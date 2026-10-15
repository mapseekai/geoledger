#!/usr/bin/env python3
"""Exercise Console auth pages against a disposable GeoLedger fixture (never production)."""
import argparse
import json
import os
from pathlib import Path
import secrets
import sys
import uuid
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', default='http://127.0.0.1:13000')
parser.add_argument('--admin-file', required=True, help='0600 JSON username/password fixture')
parser.add_argument('--chromium', help='Optional bundled Chromium executable; defaults to Playwright headless Chromium')
parser.add_argument('--output', default='target/auth-fix-three-rounds/browser')
args = parser.parse_args()
admin_path = Path(args.admin_file)
assert admin_path.stat().st_mode & 0o077 == 0, 'Fixture must have mode0600'
admin = json.loads(admin_path.read_text())
output = Path(args.output); output.mkdir(parents=True, exist_ok=True)
os.environ['TMPDIR'] = str(output.resolve())
phase = 'launch'
(output / 'result.json').write_text(json.dumps({'status': 'running', 'scenarios': []}))
completed = []
# Playwright call logs can contain form values; report only the phase and exception type.
sys.excepthook = lambda kind, value, trace: print(f'FAIL browser auth phase={phase} type={kind.__name__}', file=sys.stderr)

with sync_playwright() as playwright:
    browser = playwright.chromium.launch(headless=True, executable_path=args.chromium, chromium_sandbox=True)
    context = browser.new_context(viewport={'width': 1440, 'height': 1000})
    page = context.new_page()
    errors = []
    page.on('pageerror', lambda error: errors.append(type(error).__name__))

    def password_bounds(field, minimum):
        valid = ['甲乙丙丁', '😀😀😀', 'a' * 1024, '甲' * 341 + 'a', '😀' * 256]
        invalid = ['甲' * 342, '😀' * 256 + 'a', 'a' * 1025]
        if minimum == 12:
            invalid += ['a' * 11, '甲乙丙', '😀😀']
        else:
            valid += ['a', '甲', '😀']
        for value in valid:
            field.fill(value)
            assert field.evaluate('(input) => input.checkValidity()'), 'Valid UTF-8 password rejected'
        for value in invalid:
            field.fill(value)
            assert not field.evaluate('(input) => input.checkValidity()'), 'Invalid UTF-8 password accepted'
            assert '字节' in field.evaluate('(input) => input.validationMessage')
        field.fill('')
        assert not field.evaluate('(input) => input.checkValidity()')

    def login(p, username, password):
        p.goto(args.url + '/login')
        p.get_by_label('用户名', exact=True).fill(username)
        p.get_by_label('密码', exact=True).fill(password)
        p.get_by_role('button', name='进入控制台', exact=True).click()
        p.wait_for_url(lambda url: '/login' not in url)

    def auth(p, action, **fields):
        result = p.evaluate('''async input => {const r=await fetch('/api/auth',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(input)});return {status:r.status,data:await r.json()}}''', {'action': action, **fields})
        assert result['status'] == 200, (action, result['status'])
        return result['data']

    def confirm(p, text, password):
        dialog = p.get_by_role('dialog')
        dialog.locator('input[name="current_password"]').fill(password)
        # Keep high-risk UI writes below the service's sensitive-verification rate budget.
        p.wait_for_timeout(5100)
        dialog.get_by_role('button', name=text, exact=True).click()

    def user_page(button):
        with page.expect_response(lambda r: r.url.endswith('/api/auth') and r.request.post_data_json.get('action') == 'list_users') as loaded:
            button.click()
        response = loaded.value
        assert response.status == 200
        rows = response.json()
        if rows:
            expect(page.get_by_role('row').filter(has_text=rows[0]['id'])).to_be_visible()
        else:
            expect(page.locator('[data-slot=empty]')).to_be_visible()
            expect(page.locator('tbody tr:not(:has([data-slot=empty]))')).to_have_count(0)
        return rows

    def find_user_row(user_id):
        first = page.get_by_role('button', name='首页', exact=True)
        rows = user_page(first if first.is_enabled() else page.get_by_role('button', name='刷新', exact=True))
        for _ in range(1000):
            if any(u['id'] == user_id for u in rows):
                row = page.get_by_role('row').filter(has_text=user_id)
                expect(row).to_be_visible()
                return row
            following = page.get_by_role('button', name='下一页', exact=True)
            assert following.is_enabled(), 'Created user missing from paginated list'
            rows = user_page(following)
        raise AssertionError('User pagination did not terminate')

    def filter_users(status):
        return user_page(page.get_by_role('radiogroup', name='用户状态').get_by_role('radio', name=status, exact=True))

    phase = 'login password UTF-8 bounds'
    page.goto(args.url + '/login')
    password_bounds(page.get_by_label('密码', exact=True), 1)
    completed.append('login-password-byte-bounds')

    phase = 'password login and cookie'
    login(page, admin['username'], admin['password'])
    if '/password' in page.url:
        phase = 'initial administrator forced password change'
        initial = admin['password']
        permanent = secrets.token_urlsafe(32)
        page.get_by_label('当前密码', exact=True).fill(initial)
        page.locator('[name="new_password"]').fill(permanent)
        page.locator('[name="confirm_password"]').fill(permanent)
        page.get_by_role('button', name='保存', exact=True).click()
        page.wait_for_url('**/projects')
        admin['password'] = permanent
    page.wait_for_url('**/projects')
    cookies = context.cookies()
    cookie = next(c for c in cookies if c['name'] == 'gl_session')
    assert cookie['httpOnly'] and cookie['sameSite'] == 'Strict'
    if args.url.startswith('https:'): assert cookie['secure']
    assert 'glk_' not in cookie['value'] and admin['password'] not in cookie['value']
    assert 'gl_session' not in page.evaluate('document.cookie')

    phase = 'profile'
    page.goto(args.url + '/profile')
    expect(page.get_by_role('heading', name='个人资料', exact=True)).to_be_visible()
    identity = page.get_by_label('用户 ID', exact=True).input_value()
    page.get_by_label('显示名称', exact=True).fill('认证回归管理员')
    page.get_by_label('当前密码', exact=True).fill(admin['password'])
    page.get_by_role('button', name='保存', exact=True).click()
    expect(page.get_by_role('status')).to_contain_text('个人资料已更新')
    expect(page.get_by_label('显示名称', exact=True)).to_have_value('认证回归管理员')
    expect(page.get_by_label('当前密码', exact=True)).to_have_value('')
    expect(page.get_by_label('显示名称', exact=True)).to_have_value('认证回归管理员')
    assert auth(page, 'me')['display_name'] == '认证回归管理员'
    page.screenshot(path=str(output / 'profile.png'), full_page=True)

    phase = 'user create and forced change'
    username = 'browser_' + uuid.uuid4().hex[:10]
    temporary = ''.join(chr(0x4e00 + secrets.randbelow(20000)) for _ in range(4))
    password = ''.join(chr(0x1f600 + secrets.randbelow(64)) for _ in range(3))
    page.goto(args.url + '/users')
    page.get_by_role('button', name='新增用户', exact=True).click()
    dialog = page.get_by_role('dialog')
    dialog.get_by_label('用户名', exact=True).fill(username)
    dialog.get_by_label('显示名称', exact=True).fill('浏览器测试用户')
    password_bounds(dialog.locator('[name="new_password"]'), 12)
    password_bounds(dialog.locator('[name="current_password"]'), 1)
    dialog.locator('[name="new_password"]').fill(temporary)
    confirm(page, '确认新增用户', 'deliberately-wrong-password')
    expect(dialog.get_by_role('alert')).to_be_visible()
    expect(dialog.get_by_label('用户名', exact=True)).to_have_value(username)
    with page.expect_response(lambda r: r.url.endswith('/api/auth') and r.request.post_data_json.get('action') == 'create_user') as created:
        confirm(page, '确认新增用户', admin['password'])
    assert created.value.status == 200
    user = created.value.json()
    expect(page.get_by_role('dialog')).to_have_count(0)
    row = find_user_row(user['id'])
    expect(row).to_contain_text('待改密')
    page.screenshot(path=str(output / 'users.png'), full_page=True)
    other_context = browser.new_context()
    other = other_context.new_page()
    login(other, username, temporary)
    other.wait_for_url('**/password')
    other.goto(args.url + '/projects'); other.wait_for_url('**/password')
    denied = other.evaluate('''async()=>{const r=await fetch('/api/console',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({action:'projects'})});return r.status}''')
    assert denied == 403
    password_bounds(other.locator('[name="new_password"]'), 12)
    password_bounds(other.locator('[name="confirm_password"]'), 12)
    other.get_by_label('当前密码', exact=True).fill(temporary)
    other.locator('[name="new_password"]').fill(password)
    other.locator('[name="confirm_password"]').fill(password)
    other.get_by_role('button', name='保存', exact=True).click()
    other.wait_for_url('**/projects')
    assert other.get_by_role('link', name='用户管理', exact=True).count() == 0
    other.goto(args.url + '/users')
    expect(other.get_by_role('heading', name='用户管理', exact=True)).to_have_count(0)

    phase = 'key create once, edit expiry and source revocation'
    other.goto(args.url + '/api-keys')
    other.get_by_role('button', name='创建 API Key', exact=True).click()
    other.get_by_label('名称', exact=True).fill('browser-key')
    confirm(other, '确认创建 API Key', password)
    expect(other.get_by_label('新密钥', exact=True)).to_be_visible()
    key = other.get_by_label('新密钥', exact=True).input_value()
    assert key.startswith('glk_') and len(key) == 68
    other_context.grant_permissions(['clipboard-read', 'clipboard-write'], origin=args.url)
    other.get_by_role('button', name='复制密钥', exact=True).click()
    expect(other.get_by_role('button', name='已复制', exact=True)).to_be_visible()
    other.get_by_role('button', name='我已安全保存', exact=True).click()
    expect(other.get_by_label('新密钥', exact=True)).to_have_count(0)
    other.reload(); expect(other.get_by_label('新密钥', exact=True)).to_have_count(0)
    keyrow = other.get_by_role('row').filter(has_text='browser-key')
    keyrow.get_by_role('button', name='编辑', exact=True).click()
    other.get_by_label('名称', exact=True).fill('browser-key-updated')
    other.locator('[name="expires_at"]').fill('2099-01-01T00:00')
    confirm(other, '确认编辑 API Key', password)
    expect(other.get_by_role('dialog')).to_have_count(0)
    key_context = browser.new_context(); keypage = key_context.new_page()
    keypage.goto(args.url + '/login')
    keypage.get_by_role('tab', name='API Key 登录', exact=True).click()
    keypage.get_by_label('API Key', exact=True).fill(key)
    keypage.get_by_role('button', name='进入控制台', exact=True).click()
    keypage.wait_for_url('**/projects')
    keyrow = other.get_by_role('row').filter(has_text='browser-key-updated')
    keyrow.get_by_role('button', name='撤销', exact=True).click()
    confirm(other, '确认撤销 API Key', password)
    expect(keyrow).to_contain_text('已撤销')
    keypage.reload(); keypage.wait_for_url('**/login?expired=1')
    keyrow.get_by_role('button', name='删除', exact=True).click()
    confirm(other, '确认删除 API Key', password)
    expect(other.get_by_role('row').filter(has_text='browser-key-updated')).to_have_count(0)
    other.screenshot(path=str(output / 'api-keys.png'), full_page=True)
    key_context.close()

    phase = 'username edit, disable, enable, reset and soft deletion'
    row.get_by_role('button', name='编辑', exact=True).click()
    renamed = username + '_new'
    page.get_by_role('dialog').get_by_label('用户名', exact=True).fill(renamed)
    confirm(page, '确认编辑用户', admin['password'])
    expect(page.get_by_role('dialog')).to_have_count(0)
    row = find_user_row(user['id'])
    expect(row).to_contain_text(renamed)
    row.get_by_role('button', name='禁用', exact=True).click()
    confirm(page, '确认禁用用户', admin['password'])
    expect(page.get_by_role('dialog')).to_have_count(0)
    expect(row.get_by_role('button', name='启用', exact=True)).to_be_visible()
    filter_users('禁用')
    row = find_user_row(user['id'])
    other.goto(args.url + '/profile'); other.wait_for_url('**/login?expired=1')
    row.get_by_role('button', name='启用', exact=True).click()
    confirm(page, '确认启用用户', admin['password'])
    expect(page.get_by_role('dialog')).to_have_count(0)
    filter_users('启用')
    row = find_user_row(user['id'])
    expect(row.get_by_role('button', name='禁用', exact=True)).to_be_visible()
    row.get_by_role('button', name='重置密码', exact=True).click()
    password_bounds(page.get_by_role('dialog').locator('[name="new_password"]'), 12)
    page.get_by_role('dialog').locator('[name="new_password"]').fill(temporary)
    confirm(page, '确认重置密码', admin['password'])
    expect(row).to_contain_text('待改密')
    row.get_by_role('button', name='删除', exact=True).click()
    confirm(page, '确认删除用户', admin['password'])
    expect(page.get_by_role('dialog')).to_have_count(0)
    filter_users('已删除')
    row = find_user_row(user['id'])
    expect(row).to_contain_text('已删除')
    other_context.close()
    completed.append('multibyte-create-change-reset-confirm-password-forms')

    phase = 'create real pagination fixtures within sensitive-write rate budget'
    fixture_users = set()
    fixture_keys = set()
    prefix = 'pages_' + uuid.uuid4().hex[:10]
    for index in range(21):
        page.wait_for_timeout(5100)
        created = auth(page, 'create_user', username=f'{prefix}_{index}', password=secrets.token_urlsafe(28), current_password=admin['password'])
        fixture_users.add(created['id'])
        page.wait_for_timeout(5100)
        fixture_keys.add(auth(page, 'create_api_key', name=f'{prefix}_{index}', user_id=identity, current_password=admin['password'])['credential']['id'])
        # The one-time secret is deliberately not retained or logged.

    def all_ids(action, **fields):
        result = set()
        after = None
        for _ in range(1000):
            rows = auth(page, action, limit=100, **fields, **({'after': after} if after else {}))
            ids = [row['id'] for row in rows]
            assert not result.intersection(ids) and len(ids) == len(set(ids))
            result.update(ids)
            if len(rows) < 100:
                return result
            after = ids[-1]
        raise AssertionError('Fixture enumeration did not terminate')

    def list_page(action, trigger):
        with page.expect_response(lambda r: r.url.endswith('/api/auth') and r.request.post_data_json.get('action') == action) as loaded:
            trigger()
        response = loaded.value
        assert response.status == 200
        rows = response.json()
        expect(page.locator('tbody tr:not(:has([data-slot=empty]))')).to_have_count(len(rows))
        for row in rows:
            expect(page.locator('tbody tr').filter(has_text=row['id'])).to_be_visible()
        return rows

    def verify_pages(action, path, expected):
        rows = list_page(action, lambda: page.goto(args.url + path))
        first_ids = [row['id'] for row in rows]
        assert len(first_ids) == 20
        seen = set(first_ids)
        pages = 1
        following = page.get_by_role('button', name='下一页', exact=True)
        while following.is_enabled():
            rows = list_page(action, following.click)
            ids = [row['id'] for row in rows]
            assert len(ids) == len(set(ids)) and not seen.intersection(ids)
            seen.update(ids)
            pages += 1
            assert pages < 1000
        assert pages >= 2 and seen == expected, 'Pagination duplicated or omitted records'
        rows = list_page(action, page.get_by_role('button', name='首页', exact=True).click)
        assert [row['id'] for row in rows] == first_ids
        expect(page.get_by_role('button', name='首页', exact=True)).to_be_disabled()

    phase = 'actual user and key pagination'
    expected_users = all_ids('list_users')
    assert fixture_users <= expected_users and len(fixture_users) > 20
    verify_pages('list_users', '/users', expected_users)
    completed.append('user-pagination-multiple-pages-no-duplicates-no-omissions-home')
    expected_keys = all_ids('list_api_keys', user_id=identity)
    assert fixture_keys <= expected_keys and len(fixture_keys) > 20
    verify_pages('list_api_keys', '/api-keys', expected_keys)
    completed.append('key-pagination-multiple-pages-no-duplicates-no-omissions-home')

    def late_refresh(action, change_query, trigger=None):
        held = []
        def intercept(route):
            if route.request.post_data_json.get('action') == action and not held:
                # Capture a real old-query response, but deliver it only after the new list renders.
                held.append((route, route.fetch()))
            else:
                route.continue_()
        page.route('**/api/auth', intercept)
        try:
            (trigger or page.get_by_role('button', name='刷新', exact=True).click)()
            for _ in range(100):
                if held:
                    break
                page.wait_for_timeout(50)
            assert held, 'Old refresh was not captured'
            fresh = list_page(action, change_query)
            expected_text = page.locator('tbody').inner_text()
            with page.expect_request_finished(lambda request: request == held[0][0].request):
                held[0][0].fulfill(response=held[0][1])
            page.evaluate('() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))')
            assert page.locator('tbody').inner_text() == expected_text, 'Old refresh overwrote current query'
            expect(page.locator('tbody tr:not(:has([data-slot=empty]))')).to_have_count(len(fresh))
        finally:
            page.unroute('**/api/auth', intercept)

    phase = 'late old-owner manual refresh'
    def change_owner():
        page.get_by_label('用户 ID', exact=True).fill(sorted(fixture_users)[0])
        page.get_by_role('button', name='查看用户密钥', exact=True).click()
    late_refresh('list_api_keys', change_owner)
    completed.append('late-old-owner-response')

    phase = 'late post-write key refresh'
    list_page('list_api_keys', lambda: page.goto(args.url + '/api-keys'))
    page.locator('tbody tr').first.get_by_role('button', name='编辑', exact=True).click()
    page.get_by_role('dialog').get_by_label('名称', exact=True).fill(prefix + '_edited')
    late_refresh('list_api_keys', change_owner, lambda: confirm(page, '确认编辑 API Key', admin['password']))
    completed.append('late-post-write-owner-response')

    phase = 'late old-status and old-page manual refresh'
    list_page('list_users', lambda: page.goto(args.url + '/users'))
    late_refresh('list_users', lambda: page.get_by_role('radiogroup', name='用户状态').get_by_role('radio', name='已删除', exact=True).click())
    list_page('list_users', lambda: page.get_by_role('radiogroup', name='用户状态').get_by_role('radio', name='全部', exact=True).click())
    # A refresh clears rows, so Next is correctly disabled while it is pending.
    # Start on page two, then return Home while that page's refresh is held.
    list_page('list_users', page.get_by_role('button', name='下一页', exact=True).click)
    late_refresh('list_users', page.get_by_role('button', name='首页', exact=True).click)
    completed.append('late-old-status-and-page-response')

    phase = 'late post-write user refresh'
    fixture_row = find_user_row(sorted(fixture_users)[0])
    fixture_row.get_by_role('button', name='编辑', exact=True).click()
    page.get_by_role('dialog').get_by_label('显示名称', exact=True).fill('pagination fixture edited')
    late_refresh('list_users', lambda: page.get_by_role('radiogroup', name='用户状态').get_by_role('radio', name='已删除', exact=True).click(), lambda: confirm(page, '确认编辑用户', admin['password']))
    completed.append('late-post-write-status-response')

    phase = 'csrf, logout and fixation'
    assert context.request.post(args.url + '/api/auth', data={'action':'me'}, headers={'Origin':'https://invalid.example'}).status == 403
    assert context.request.post(args.url + '/api/session', data=admin, headers={'Origin':'https://invalid.example'}).status == 403
    assert context.request.delete(args.url + '/api/session', headers={'Origin':'https://invalid.example','Content-Type':'application/json'}).status == 403
    assert context.request.delete(args.url + '/api/session', headers={'Content-Type':'application/json'}).status == 403
    assert auth(page, 'me')['id'] == identity
    prior = next(c for c in context.cookies() if c['name']=='gl_session')['value']
    page.get_by_role('button', name='退出登录', exact=True).click();page.wait_for_url('**/login')
    replay = browser.new_context()
    replay.add_cookies([{'name':'gl_session','value':prior,'url':args.url}])
    assert replay.request.post(args.url+'/api/console',data={'action':'projects'},headers={'Origin':args.url}).status == 401
    login(page, admin['username'], admin['password'])
    assert next(c for c in context.cookies() if c['name']=='gl_session')['value'] != prior
    page.set_viewport_size({'width':390,'height':844});page.goto(args.url+'/profile')
    page.screenshot(path=str(output/'mobile-profile.png'),full_page=True)
    assert not errors, 'Unexpected page errors'
    (output/'result.json').write_text(json.dumps({'status':'passed','admin_id':identity,'scenarios':['password-login','httpOnly-cookie','profile','user-crud','forced-change','key-login','key-crud','source-session-revoke','stable-id','disable-session','reset','csrf','logout','fixation','mobile','copy-key','user-status-filters','logout-csrf','wrong-current-password'] + completed},indent=2))
    replay.close();browser.close()
    print('PASS Console auth browser lifecycle; credentials omitted, screenshots contain no key')
