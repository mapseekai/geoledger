#!/usr/bin/env python3
"""Disposable native restore + restricted-role binding drill. Requires Docker."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request
import uuid
from benchmark import ports, stop
from auth_test_support import provision, auth_call
import secrets
from benchmark_incremental import verify_source_crud


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--server', type=Path, default=Path('target/debug/geoledger-server'))
    p.add_argument('--image', default='postgis/postgis@sha256:01a6a70e41e6c4467c8f55f6063555ed72db2d6662cd0d571040d42eadaeb6f6')
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--web', action='store_true')
    p.add_argument('--chromium')
    a = p.parse_args()
    if a.web and not a.chromium: p.error("--web requires --chromium")
    a.output.mkdir(parents=True, exist_ok=False)
    dbport, http, grpc, webport = ports(4)
    container = 'geoledger-release-' + uuid.uuid4().hex[:12]
    processes = []
    env = {k:v for k,v in os.environ.items() if not k.startswith('GL_')}
    env.update(GL_DATABASE_URL=f'postgresql://geoledger_app@127.0.0.1:{dbport}/geoledger_test?sslmode=disable')
    def sql(statement, database='geoledger_test'):
        return subprocess.check_output(['docker','exec','-i',container,'psql','-U','postgres','-d',database,'-At','-v','ON_ERROR_STOP=1'], input=statement, text=True).strip()
    def start(cmd, name, url):
        with (a.output/(name+'.log')).open('w') as log:
            proc = subprocess.Popen(cmd,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        processes.append(proc)
        for _ in range(120):
            try:
                with urllib.request.urlopen(url,timeout=1): return proc
            except OSError: pass
            if proc.poll() is not None: raise RuntimeError(name+' exited; inspect log')
            time.sleep(.25)
        raise RuntimeError(name+' readiness timeout')
    try:
        subprocess.run(['docker','run','--rm','-d','--name',container,'-e','POSTGRES_HOST_AUTH_METHOD=trust','-e','POSTGRES_DB=geoledger_test','-p',f'127.0.0.1:{dbport}:5432',a.image],check=True,stdout=subprocess.DEVNULL)
        for _ in range(120):
            if subprocess.run(['docker','exec',container,'pg_isready','-h','127.0.0.1','-U','postgres','-d','geoledger_test'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode == 0: break
            time.sleep(.5)
        subprocess.run(['docker','cp','deploy/postgis/initialize.sh',container+':/tmp/initialize.sh'],check=True)
        subprocess.run(['docker','exec','-e','POSTGRES_USER=postgres','-e','POSTGRES_DB=geoledger_test','-e','GL_DB_APP_PASSWORD=disposable-only',container,'sh','/tmp/initialize.sh'],check=True,stdout=subprocess.DEVNULL)
        sql("CREATE SCHEMA business AUTHORIZATION geoledger_app; SET ROLE geoledger_app; CREATE TABLE business.roads(id text PRIMARY KEY, name text, geom geometry(LineString,4326)); INSERT INTO business.roads VALUES('one','original',ST_GeomFromText('LINESTRING(104 35,105 36)',4326));")
        sql('CREATE SCHEMA private; CREATE TABLE private.unrelated(secret text)')
        assert sql("SELECT has_table_privilege('geoledger_app','private.unrelated','SELECT')") == 'f'
        assert sql("SELECT rolsuper OR rolcreatedb OR rolcreaterole FROM pg_roles WHERE rolname='geoledger_app'") == 'f'
        base=f'http://127.0.0.1:{http}'
        cmd=[str(a.server.resolve()),'--storage','postgis','--data-dir',str(a.output/'data'),'--http',f'127.0.0.1:{http}','--grpc',f'127.0.0.1:{grpc}']
        server=start(cmd,'server',base+'/ready')
        identity=provision(base,a.output/'data',a.output/'api-key')
        token=identity['api_key']
        def call(op, body, credential=None):
            req=urllib.request.Request(base+'/api/v1/'+op,data=json.dumps(body).encode(),headers={'Authorization':'Bearer '+(credential or token),'Content-Type':'application/json'})
            with urllib.request.urlopen(req,timeout=30) as r:return json.load(r)
        project=call('create_project',{'name':'restore drill'})['project']
        dataset=call('create_dataset',{'project':project,'name':'roads','postgis_table':{'schema':'business','table':'roads','id_column':'id','geometry_column':'geom'}})['dataset']
        def publish(value):
            workspace=call('create_workspace',{'project':project})['workspace']
            v=call('save',{'project':project,'workspace':workspace,'expected_workspace_version':0,'edits':[{'dataset':dataset,'feature_id':'one','feature':{'type':'Feature','id':'one','properties':{'name':value},'geometry':{'type':'LineString','coordinates':[[104,35],[106,37]]}}}]})['version']
            return call('publish',{'project':project,'workspace':workspace,'expected_workspace_version':v,'request_id':str(uuid.uuid4()),'message':value})
        temporary=secrets.token_urlsafe(32)
        user=auth_call(base,'create_user',{'username':'restore-member','password':temporary,'current_password':identity['password']},identity['session'])
        login=auth_call(base,'login_password',{'username':'restore-member','password':temporary})
        password=secrets.token_urlsafe(32)
        changed=auth_call(base,'change_password',{'current_password':temporary,'new_password':password},login['session'])
        member=auth_call(base,'create_api_key',{'name':'restore','current_password':password},changed['session'])
        call('set_member',{'project':project,'subject':user['id'],'role':'viewer'})
        publish('before backup')
        before=call('features',{'project':project,'dataset':dataset})
        audit=call('audit',{'project':project,'limit':1000})
        with (a.output/'backup.dump').open('wb') as output:
            subprocess.run(['docker','exec',container,'pg_dump','-U','postgres','-Fc','geoledger_test'],stdout=output,check=True)
        stop(server)
        started=time.monotonic()
        with (a.output/'backup.dump').open('rb') as backup:
            subprocess.run(['docker','exec','-i',container,'pg_restore','-U','postgres','--exit-on-error','--clean','--if-exists','-d','geoledger_test'],stdin=backup,check=True)
        server=start(cmd,'restored',base+'/ready')
        assert call('features',{'project':project,'dataset':dataset}) == before
        assert call('audit',{'project':project,'limit':1000}) == audit
        assert call('features',{'project':project,'dataset':dataset},member['api_key']) == before
        restore_seconds=time.monotonic()-started
        publish('after restore')
        assert sql("SELECT name FROM business.roads WHERE id='one'",'geoledger_test') == 'after restore'
        assert call('features',{'project':project,'dataset':dataset,'revision':2}) == before
        sql('SET ROLE geoledger_app; CREATE TABLE business.source(id text PRIMARY KEY, attributes jsonb, geom geometry(LineString,4326))','geoledger_test')
        source=call('create_dataset',{'project':project,'name':'batch source','postgis_table':{'schema':'business','table':'source','id_column':'id','geometry_column':'geom'}})['dataset']
        verify_source_crud(call,lambda statement:sql(statement,'geoledger_test'),project,source,{'type':'Feature','id':'sample','properties':{'attributes':{'exact':18446744073709551615}},'geometry':{'type':'LineString','coordinates':[[104,35],[106,37]]}})
        call('delete_project',{'project':project,'confirm_name':'restore drill'})
        assert sql("SELECT name FROM business.roads WHERE id='one'",'geoledger_test') == 'after restore'
        assert sql("SELECT count(*) FROM pg_trigger WHERE tgrelid='business.roads'::regclass AND NOT tgisinternal",'geoledger_test') == '0'
        if a.web:
            env.update(GL_WEB_ENDPOINT=f'http://127.0.0.1:{http}',GL_WEB_ORIGIN=f'http://127.0.0.1:{webport}',GL_WEB_SESSION_SECRET=secrets.token_urlsafe(48))
            start(['npm','--prefix','web','start','--','--port',str(webport)],'web',env['GL_WEB_ORIGIN']+'/login')
            subprocess.run([os.sys.executable,'scripts/test-console-postgis.py','--url',env['GL_WEB_ORIGIN'],'--api-key-file',str(a.output/'api-key'),'--chromium',a.chromium,'--screenshots',str(a.output/'screenshots')],check=True,timeout=180)
        (a.output/'result.json').write_text(json.dumps({'passed':True,'restore_seconds':restore_seconds,'restricted_role':True,'native_restore':True,'source_survives_unbind':True,'source_batches':[1,100,500]},indent=2))
        print('PASS restricted role, native restore, history, audit, continued publication and source unbinding; restore seconds:',round(restore_seconds,3))
    finally:
        for proc in reversed(processes): stop(proc)
        subprocess.run(['docker','rm','-f',container],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)

if __name__ == '__main__': main()
