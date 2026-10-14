#!/usr/bin/env python3
"""Check private Console BFF auth and exact business transport on a disposable service."""
import argparse
import http.cookiejar
import json
from pathlib import Path
import secrets
import time
import urllib.error
import urllib.request
import uuid

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url',default='http://127.0.0.1:13001')
parser.add_argument('--admin-file',required=True)
parser.add_argument('--output',default='target/auth-refactor-bff/result.json')
args=parser.parse_args()
source=Path(args.admin_file)
assert source.stat().st_mode&0o077==0,'Use a0600 disposable credential fixture'
admin=json.loads(source.read_text())
class Browser:
    def __init__(self):
        self.jar=http.cookiejar.CookieJar()
        self.client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
    def call(self,path,payload=None,expected=200,origin=None,headers=None,method='POST'):
        request=urllib.request.Request(args.url+path,data=None if payload is None else json.dumps(payload).encode(),method=method,headers={'Content-Type':'application/json','Origin':origin or args.url,**(headers or {})})
        for attempt in range(12):
            try:response=self.client.open(request)
            except urllib.error.HTTPError as error:response=error
            if response.status != 429 or expected == 429: break
            response.close();time.sleep(5.1)
        raw=response.read().decode()
        assert response.status==expected,(path,response.status,expected)
        return json.loads(raw) if raw.startswith(('{','[')) else raw
    def auth(self,action,expected=200,**fields):return self.call('/api/auth',{'action':action,**fields},expected)
    def business(self,action,expected=200,**fields):return self.call('/api/console',{'action':action,**fields},expected)
    def cookie(self):return next(c.value for c in self.jar if c.name=='gl_session')

b=Browser()
login=b.call('/api/session',admin)
assert set(login)=={'user'} and not login['user']['must_change_password']
user_id=login['user']['id']
assert 'session' not in login and admin['password'] not in b.cookie()
for path in ['/projects','/users','/profile','/password','/api-keys']:
    html=b.call(path,method='GET');assert 'GeoLedger' in html,path
for path,payload in [('/api/auth',{'action':'me'}),('/api/console',{'action':'projects'}),('/api/session',admin)]:
    b.call(path,payload,403,origin='https://invalid.example')
b.call('/api/session',{},403,origin='https://invalid.example',method='DELETE')
assert b.auth('me')['id']==user_id
b.call('/api/console',{'action':'projects'},400,headers={'Authorization':'Bearer invalid'})
b.call('/api/console',{'action':'projects'},401,headers={'Cookie':'gl_session=invalid; gl_session=invalid2'})

username='bff_'+uuid.uuid4().hex[:10]
temporary=secrets.token_urlsafe(30)
new_password=secrets.token_urlsafe(30)
created=b.auth('create_user',username=username,password=temporary,current_password=admin['password'])
assert created['must_change_password'] and 'password_hash' not in created
member=Browser();member.call('/api/session',{'username':username,'password':temporary})
member.business('projects',403)
assert '请先修改' in member.call('/projects',method='GET')
member.auth('create_api_key',403,name='blocked',current_password=temporary)
changed=member.auth('change_password',current_password=temporary,new_password=new_password)
assert not changed['user']['must_change_password'] and 'session' not in changed
member.auth('list_users',403)
member.auth('update_user',403,user_id=user_id,is_admin=False,current_password=new_password)

project=b.business('createProject',name='BFF exact '+uuid.uuid4().hex[:8])['id']
b.business('setMember',400,project=project,subject=created['id'],role='viewer')
b.business('setMember',403,project=project,subject=created['id'],role='viewer',currentPassword='wrong-password')
b.business('setMember',project=project,subject=created['id'],role='viewer',currentPassword=admin['password'])
assert member.business('project',project=project)['role']=='viewer'
dataset=b.business('createDataset',project=project,name='points',geometryType='point',coordinateDimension=2,crs='EPSG:4326')['id']
workspace=b.business('createWorkspace',project=project)['id']
# Validate malformed feature strings at the BFF before forwarding any business write.
for action in ('save', 'resolve', 'rebase'):
    fields={'project':project,'workspace':workspace,'version':'0','edits':[{'dataset':dataset,'featureId':'bad','feature':'{'}]}
    if action != 'save': fields['head']='0'
    invalid=b.business(action,400,**fields)
    assert invalid['error']['code']=='invalid_argument' and not invalid['error'].get('uncertain',False)
# Invalid UTF-8 must not become a replacement-character project name.
projects_before=b.business('projects')
raw=urllib.request.Request(args.url+'/api/console',data=b'{"action":"createProject","name":"'+bytes([255])+b'"}',headers={'Content-Type':'application/json','Origin':args.url})
try: response=b.client.open(raw)
except urllib.error.HTTPError as error: response=error
assert response.status==400
response.close()
assert b.business('projects')==projects_before

feature='{"type":"Feature","id":"exact","properties":{"large":18446744073709551615,"snake_case":"keep","fraction":1.00000000000000001e18},"geometry":{"type":"Point","coordinates":[104,35]}}'
saved=b.business('save',project=project,workspace=workspace,version='0',edits=[{'dataset':dataset,'featureId':'exact','feature':feature}])
assert saved['version']=='1'
collection=b.business('features',project=project,dataset=dataset,workspace=workspace)
assert collection['features'][0]['properties']['large']==18446744073709551615
assert collection['features'][0]['properties']['snake_case']=='keep'
publication={'project':project,'workspace':workspace,'version':saved['version'],'requestId':str(uuid.uuid4()),'message':'BFF regression'}
published=b.business('publish',**publication)
assert published['revision']=='1'
assert b.business('publish',**publication)['revision']=='1'
assert b.business('history',project=project)[0]['subject']==user_id
assert b.business('audit',project=project)['events']
assert b.business('commitSummary',project=project,revision='1')['total']['added']=='1'

issued=b.auth('create_api_key',name='source-session-test',current_password=admin['password'])
assert isinstance(issued['api_key'],str) and issued['api_key'].startswith('glk_')
assert 'digest' not in issued['credential']
key_browser=Browser();key_browser.call('/api/session',{'api_key':issued['api_key']})
key_browser.auth('create_api_key',403,name='requires-password')
b.auth('revoke_api_key',key_id=issued['credential']['id'],current_password=admin['password'])
key_browser.business('projects',401)
b.auth('delete_api_key',key_id=issued['credential']['id'],current_password=admin['password'])
member.auth('update_user',user_id=created['id'],username=username+'_new',current_password=new_password)
assert member.auth('me')['id']==created['id']
assert any(m['subject']==created['id'] for m in b.business('members',project=project))
b.auth('reset_password',user_id=created['id'],new_password=secrets.token_urlsafe(30),current_password=admin['password'])
member.auth('me',401)

copied=b.cookie();b.call('/api/session',{},method='DELETE')
Browser().call('/api/console',{'action':'projects'},401,headers={'Cookie':'gl_session='+copied})
b.call('/api/session',admin);assert b.cookie()!=copied
output=Path(args.output);output.parent.mkdir(parents=True,exist_ok=True)
output.write_text(json.dumps({'status':'passed','scenarios':['malformed-feature-client-errors','strict-UTF8-body','password-login','SSR-auth-pages','csrf','mixed-credentials','duplicate-cookie','forced-change','user-isolation','membership-password','exact-GeoJSON','publication-retry','RBAC','audit','key-login','source-key-revocation','stable-user-id','password-reset-invalidation','logout','session-fixation']},indent=2))
print('PASS private BFF auth/security/business lifecycle; no credentials emitted')
