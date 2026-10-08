#!/usr/bin/env python3
"""Browser integration against a disposable server (requires playwright + Chromium).

python3 scripts/test-console.py --url http://127.0.0.1:7895 \
  --token-file /path/to/disposable/tokens.json --chromium /usr/bin/chromium
Creates a new project and test data. Never run against a production instance.
"""
import argparse
import json
import time
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', required=True)
parser.add_argument('--token-file', type=Path, required=True)
parser.add_argument('--chromium')
parser.add_argument('--screenshots', type=Path)
args = parser.parse_args()
token = json.loads(args.token_file.read_text())[0]['token']
if args.screenshots:
    args.screenshots.mkdir(parents=True, exist_ok=True)

with sync_playwright() as pw:
    browser = pw.chromium.launch(**({'executable_path': args.chromium} if args.chromium else {}),
                                 args=['--no-sandbox', '--disable-dev-shm-usage'])
    context = browser.new_context(viewport={'width': 1440, 'height': 960})
    page = context.new_page()
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.goto(args.url)
    expect(page.locator('#login')).to_be_visible()
    expect(page.locator('#app')).to_be_hidden()
    assert page.request.get(args.url + '/logo.png').status == 200
    if args.screenshots:
        page.screenshot(path=str(args.screenshots / 'login.png'), full_page=True)
    page.locator('#token').fill('invalid-token')
    page.locator('#connect').click()
    expect(page.locator('#login-error')).to_contain_text('令牌无效')
    expect(page.locator('#app')).to_be_hidden()
    page.locator('#token').fill(token)
    page.locator('#token').press('Enter')
    expect(page.locator('#app')).to_be_visible()
    expect(page.locator('#connection')).to_contain_text('sqlite')

    page.locator('[data-page="projects"]').click()
    expect(page.locator('#page-title')).to_have_text('项目')
    page.locator('#primary-action').click()
    project_name = f'城市空间数据 · {int(time.time())}'
    page.locator('#new-name').fill(project_name)
    page.locator('#create-submit').click()
    expect(page.locator('#summary-name')).to_have_text(project_name)
    expect(page.locator('#inventory')).to_contain_text('暂无数据集')
    project_id = page.locator('#project').input_value()

    page.locator('#primary-action').click()
    page.locator('#new-name').fill('城市道路')
    page.locator('#new-name').press('Enter')
    expect(page.locator('#inventory')).to_contain_text('城市道路')
    page.locator('#inventory').get_by_role('button', name='浏览要素').click()
    expect(page.locator('#primary-action')).to_be_disabled()
    page.locator('#choose-workspace').click()
    page.locator('#primary-action').click()
    expect(page.locator('#inventory')).to_contain_text('编辑中')
    page.locator('#inventory').get_by_role('button', name='使用工作区').click()
    expect(page.locator('#page-title')).to_have_text('城市道路')
    page.locator('#primary-action').click()
    page.locator('#feature-id').fill('road-001')
    feature = '{"type":"Feature","properties":{"名称":"滨江大道","精确值":9007199254740993},"geometry":{"type":"LineString","coordinates":[[120,30],[120.01,30.01]]}}'
    page.locator('#feature-json').fill(feature)
    with page.expect_request('**/api/v1/save') as sent:
        page.locator('#save-feature').click()
    assert '9007199254740993' in sent.value.post_data
    expect(page.locator('#inventory')).to_contain_text('road-001')
    expect(page.locator('#version')).to_have_value('1')
    page.locator('#inventory').get_by_role('button', name='查看要素').click()
    expect(page.locator('#detail-json')).to_contain_text('9007199254740993')
    page.locator('[data-close="detail-dialog"]').click()
    page.locator('#show-diff').click()
    expect(page.locator('#inventory')).to_contain_text('新增')

    # A lost response must retain the same publication identity and body.
    publications = []
    def lose_first_response(route):
        publications.append(route.request.post_data)
        response = route.fetch()
        if len(publications) == 1:
            route.abort('failed')
        else:
            route.fulfill(response=response)
    page.route('**/api/v1/publish', lose_first_response)
    page.locator('#publish-draft').click()
    page.locator('#publish-message').fill('新增城市道路要素')
    page.locator('#confirm-publish').click()
    expect(page.locator('#publish-error')).to_contain_text('网络请求未完成')
    expect(page.locator('#publish-message')).to_be_disabled()
    page.locator('#publish-dialog').get_by_role('button', name='取消', exact=True).click()
    page.locator('[data-page="advanced"]').click()
    assert json.loads(page.locator('#request').input_value()) == json.loads(publications[0])
    expect(page.locator('#endpoint')).to_have_text('POST /api/v1/publish')
    page.locator('[data-page="datasets"]').click()
    page.locator('#inventory').get_by_role('button', name='浏览要素').click()
    page.locator('#publish-draft').click()
    expect(page.locator('#publish-message')).to_be_disabled()
    page.locator('#confirm-publish').click()
    expect(page.locator('#publish-dialog')).not_to_be_visible()
    expect(page.locator('#summary-head')).to_have_text('v1')
    assert len(publications) == 2 and publications[0] == publications[1]
    page.unroute('**/api/v1/publish', lose_first_response)
    expect(page.locator('#inventory')).to_contain_text('road-001')
    page.locator('[data-page="history"]').click()
    expect(page.locator('#inventory')).to_contain_text('新增城市道路要素')
    page.locator('#inventory').get_by_role('button', name='查看变更').click()
    expect(page.locator('#detail-json')).to_contain_text('9007199254740993')
    page.locator('[data-close="detail-dialog"]').click()
    page.locator('[data-page="audit"]').click()
    expect(page.locator('#inventory')).to_contain_text('publish')
    page.locator('[data-page="access"]').click()
    page.locator('#member-subject').fill('console-viewer')
    page.locator('#save-member').click()
    expect(page.locator('#notice')).to_contain_text('成员权限已保存')

    # Populate enough real datasets to verify cursor navigation and local filtering.
    headers = {'Authorization': f'Bearer {token}'}
    for name in ['建筑轮廓', '行政区划', '地形高程', '水系与湖泊', '公共设施']:
        response = page.request.post(args.url + '/api/v1/create_dataset', headers=headers,
                                     data={'project': project_id, 'name': name})
        assert response.ok
    page.locator('[data-page="datasets"]').click()
    expect(page.locator('#inventory tbody tr')).to_have_count(6)
    expect(page.locator('#notice')).to_be_empty(timeout=10000)
    if args.screenshots:
        page.screenshot(path=str(args.screenshots / 'console.png'), full_page=True)
    for i in range(16):
        response = page.request.post(args.url + '/api/v1/create_dataset', headers=headers,
                                     data={'project': project_id, 'name': f'分页测试 {i:02}'})
        assert response.ok
    page.locator('#refresh').click()
    expect(page.locator('#inventory tbody tr')).to_have_count(20)
    page.locator('#next-page').click()
    expect(page.locator('#page-number')).to_have_text('2')
    expect(page.locator('#inventory tbody tr')).to_have_count(2)
    page.locator('#previous-page').click()
    expect(page.locator('#page-number')).to_have_text('1')
    page.locator('#filter').fill('不存在的名称')
    expect(page.locator('#inventory')).to_contain_text('没有匹配的记录')
    page.locator('#filter').fill('')
    expect(page.locator('#inventory tbody tr')).to_have_count(20)
    assert page.locator('body').evaluate('(body) => body.scrollWidth <= innerWidth')

    # Service expiry logs out instead of leaving protected resource data on screen.
    page.route('**/api/v1/list_datasets', lambda route: route.fulfill(status=401, content_type='application/json', body='{"error":{"message":"expired"}}'))
    page.locator('#refresh').click()
    expect(page.locator('#login')).to_be_visible()
    expect(page.locator('#token')).to_have_value('')
    expect(page.locator('#inventory')).to_be_empty()
    expect(page.locator('#response')).to_be_empty()
    page.unroute('**/api/v1/list_datasets')
    page.locator('#token').fill(token)
    page.locator('#connect').click()
    expect(page.locator('#app')).to_be_visible()
    page.locator('#logout').click()
    expect(page.locator('#login')).to_be_visible()
    expect(page.locator('#token')).to_have_value('')
    page.reload()
    expect(page.locator('#login')).to_be_visible()
    assert page.evaluate('Object.keys(localStorage).length + Object.keys(sessionStorage).length') == 0
    assert not context.cookies()

    page.set_viewport_size({'width': 390, 'height': 844})
    assert page.locator('body').evaluate('(body) => body.scrollWidth <= innerWidth')
    if args.screenshots:
        page.screenshot(path=str(args.screenshots / 'login-mobile.png'), full_page=True)
    page.locator('#token').fill(token)
    page.locator('#connect').click()
    expect(page.locator('#app')).to_be_visible()
    page.locator('#open-menu').click()
    expect(page.locator('#sidebar')).to_be_visible()
    page.locator('[data-page="service"]').click()
    expect(page.locator('#service-page')).to_be_visible()
    expect(page.locator('#sidebar')).to_be_hidden()
    assert page.locator('body').evaluate('(body) => body.scrollWidth <= innerWidth')
    if args.screenshots:
        page.screenshot(path=str(args.screenshots / 'console-mobile.png'), full_page=True)
    assert not errors, errors
    browser.close()
    print('PASS: login, invalid/expired token, resource CRUD, exact integers, publication retry, history/audit/access, cursor pagination, filtering, logout, mobile, no JavaScript errors')
