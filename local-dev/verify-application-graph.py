#!/usr/bin/env python3
"""Verify onboarding against real Backstage catalog entities and native graphs."""
import argparse
import json
import os
from pathlib import Path
import time
import urllib.error
import urllib.request

parser=argparse.ArgumentParser()
parser.add_argument('--base-url',default='http://localhost:17007/api/application-graph')
parser.add_argument('--token',default=os.environ.get('APPLICATION_GRAPH_BACKSTAGE_TOKEN','application-graph-local-development'))
parser.add_argument('--verify-existing',action='store_true')
parser.add_argument('--verify-catalog-edits',action='store_true')
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]
output=root/'local-dev/.generated/application-graph-results.json'

def request(path,body=None,base=None):
    req=urllib.request.Request((base or args.base_url)+path,headers={'Authorization':'Bearer '+args.token,'Content-Type':'application/json'},
        data=None if body is None else json.dumps(body).encode())
    with urllib.request.urlopen(req,timeout=30) as response:
        payload=response.read()
        return json.loads(payload) if payload.strip() else None

started=time.monotonic()
while True:
    try:
        if request('/status')['graphServiceAvailable']:break
    except (OSError,ValueError):pass
    assert time.monotonic()-started<180,'Backstage application-graph backend did not become available'
    time.sleep(1)

# RHDH's local in-memory catalog hydrates asynchronously after the backend starts.
catalog=args.base_url.rsplit('/api/',1)[0]+'/api/catalog'
started=time.monotonic()
while True:
    try:
        for name in ['spring-quoters','spring-quotes-client']:
            entity=request('/entities/by-name/component/default/'+name,base=catalog)
            assert entity['metadata']['name']==name
        break
    except (OSError,ValueError):
        assert time.monotonic()-started<180,'Catalog applications did not become available'
        time.sleep(1)

if args.verify_catalog_edits:
    before=json.loads(output.read_text())
    descriptor=root/'local-dev/application-catalog.yaml'
    original=descriptor.read_text()
    change='  consumesApis:\n    - quotes-api'
    assert original.count(change)==1
    locations=request('/entities?filter=kind=location',base=catalog)
    location=next(e for e in locations if e.get('spec',{}).get('target','').endswith('application-catalog.yaml'))
    location_ref='location:'+location['metadata'].get('namespace','default')+'/'+location['metadata']['name']
    def refresh(): request('/refresh',{'entityRef':location_ref},base=catalog)
    def wait_relationship(present):
        deadline=time.monotonic()+90
        while True:
            component=request('/entities/by-name/component/default/spring-quotes-client',base=catalog)
            actual=any(r['type']=='consumesApi' and r['targetRef']=='api:default/quotes-api' for r in component.get('relations',[]))
            if actual==present:return
            assert time.monotonic()<deadline,'Catalog relationship change did not process'
            time.sleep(.5)
    try:
        descriptor.write_text(original.replace(change,'  consumesApis: []'))
        refresh();wait_relationship(False)
        assert request('/connections')['connections']==[]
        current={a['entityRef']:a for a in request('/applications')['applications']}
        assert all(current[ref]['graphVersion']==old['graphVersion'] for ref,old in before['applications'].items())
    finally:
        descriptor.write_text(original)
        refresh();wait_relationship(True)
    assert request('/connections')['connections']==before['connections']
    before['liveCatalogEdits']='passed without graph reindexing'
    output.write_text(json.dumps(before,indent=2)+'\n')
    print('Removing/restoring consumesApis changed connections without changing either stored graph.')
    raise SystemExit(0)

if args.verify_existing:
    before=json.loads(output.read_text())
    after={a['entityRef']:a for a in request('/applications')['applications']}
    for ref,old in before['applications'].items():
        assert ref in after
        for field in ['graphVersion','commit','snapshotSha256','sourceSha256']:
            assert after[ref][field]==old[field],f'{ref}: persisted {field} changed'
        graph=request('/applications/'+old['applicationKey']+'/nodes?limit=10')
        assert graph['nodes'] and graph['graphVersion']==old['graphVersion']
    links=request('/connections')['connections']
    assert links==before['connections']
    before['restartPersistence']='passed'
    output.write_text(json.dumps(before,indent=2)+'\n')
    print('Stored graph identities, native node reads, and connections survived restart.')
    raise SystemExit(0)

previous=json.loads(output.read_text()) if output.is_file() else None
jobs=[]
for name in ['spring-quoters','spring-quotes-client']:
    job=request('/onboard',{'entityRef':'component:default/'+name})
    deadline=time.monotonic()+240
    while job['state'] in ['queued','running']:
        assert time.monotonic()<deadline,'Onboarding timed out'
        time.sleep(.5)
        job=request('/jobs/'+job['id'])
    assert job['state']=='succeeded',job
    jobs.append(job)
    print(name,'onboarded',job['graphVersion'],flush=True)

apps={a['entityRef']:a for a in request('/applications')['applications']}
provider=apps['component:default/spring-quoters']
consumer=apps['component:default/spring-quotes-client']
assert provider['providesApis']==consumer['consumesApis']==['api:default/quotes-api']
for app in [provider,consumer]:
    assert app['nodeCount']>0 and app['edgeCount']>0 and len(app['commit'])==40
    nodes=request('/applications/'+app['applicationKey']+'/nodes?limit=1000')
    assert nodes['graphVersion']==app['graphVersion'] and nodes['nodes']
    edges=request('/applications/'+app['applicationKey']+'/edges?limit=1000')
    assert edges['edges']
    assert any(edge['type']=='Calls' for edge in edges['edges'])
    assert all(not node.get('file_path','').endswith('Test.java') for node in nodes['nodes'] if node.get('file_path'))
assert any(o['path']=='/api/random' and o['methods']==['GET'] for o in provider['operations'])
assert any(o['path']=='/api/random' and o['methods']==['GET'] for o in consumer['operations'])
connections=request('/connections')['connections']
assert len(connections)==1,connections
connection=connections[0]
assert connection['status']=='source-linked',connection
assert connection['provider']==provider['entityRef'] and connection['consumer']==consumer['entityRef']
matches=connection['operationMatches']
assert any(m['method']=='GET' and m['path']=='/api/random' and m['callee']['symbol']['qualifiedName'].endswith('.getRandomOne') for m in matches),matches
for match in matches:
    assert match['caller']['graphVersion']==consumer['graphVersion']
    assert match['callee']['graphVersion']==provider['graphVersion']
    for side,app in [('caller',consumer),('callee',provider)]:
        node_id=match[side]['symbol']['nodeId']
        nodes=request('/applications/'+app['applicationKey']+'/nodes?nodeId='+node_id)
        assert len(nodes['nodes'])==1 and nodes['nodes'][0]['id']==node_id
if previous:
    for ref,old in previous['applications'].items():
        path='/applications/'+old['applicationKey']
        historic=request(path+'/versions/'+old['graphVersion'])
        assert historic['snapshotSha256']==old['snapshotSha256']
        historic_nodes=request(path+'/nodes?graphVersion='+old['graphVersion']+'&limit=1')
        assert historic_nodes['graphVersion']==old['graphVersion'] and historic_nodes['nodes']
    try:
        request('/applications/'+provider['applicationKey']+'/versions/'+consumer['graphVersion'])
        raise AssertionError('Graph version exposed through the wrong application')
    except urllib.error.HTTPError as e: assert e.code==404
result={'status':'passed','jobs':jobs,'applications':apps,'connections':connections,'checks':[
    'catalog-authorized onboarding','real Git repository revisions','native snapshot node/Calls reads',
    'production Java source selection','provider and consumer operation discovery','standard catalog API relationship',
    'cross-service HTTP transition with separately scoped graph symbols'],
    'previousVersions': 'passed' if previous else 'not exercised'}
output.parent.mkdir(parents=True,exist_ok=True)
output.write_text(json.dumps(result,indent=2)+'\n')
print('Verified GET /api/random: consumer invocation -> Quoters.getRandomOne.')
