const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const express = require('express');
const build = path.resolve(process.env.APPLICATION_GRAPH_TEST_BUILD || 'local-dev/.generated/application-graph-tests');
const { entityRef, source, onboardingRequest, HttpError } = require(path.join(build,'catalog.js'));
const { createRouter } = require(path.join(build,'router.js'));
const key = 'a'.repeat(64);
const entity = {
  kind:'Component',metadata:{name:'client',namespace:'payments',annotations:{'backstage.io/source-location':'url:https://github.com/acme/client/tree/main/service'}},
  spec:{consumesApis:['wrong-api']},relations:[{type:'consumesApi',targetRef:'api:payments/quotes'}],
};
async function host(t, options) {
  const app=express(); app.use(createRouter(options));
  const server=await new Promise(resolve=>{const s=app.listen(0,'127.0.0.1',()=>resolve(s));});
  t.after(()=>new Promise(resolve=>server.close(resolve)));
  return `http://127.0.0.1:${server.address().port}`;
}
test('catalog normalization preserves namespace and authoritative processed relations',()=>{
  assert.equal(entityRef('Quotes','api','payments'),'api:payments/quotes');
  assert.equal(entityRef('Component:payments/Client'),'component:payments/client');
  assert.throws(()=>entityRef('../outside'));
  const request=onboardingRequest(entity,'component:payments/client');
  assert.deepEqual(request.consumesApis,['api:payments/quotes']);
  assert.deepEqual(request.source,{repositoryUrl:'https://github.com/acme/client.git',revision:'main',subPath:'service'});
});
test('source parsing supports root repositories and rejects embedded credentials',()=>{
  assert.equal(source('url:https://github.com/acme/service').revision,'HEAD');
  assert.deepEqual(source('url:https://gitlab.example/acme/platform/-/tree/release/app'),{repositoryUrl:'https://gitlab.example/acme/platform.git',revision:'release',subPath:'app'});
  assert.throws(()=>source('url:https://token@github.com/acme/app'));
  assert.throws(()=>source('file:/private/source'));
});
test('onboarding ignores caller-supplied sources and API relations',async t=>{
  let sent;
  const base=await host(t,{readEntity:async(_request,ref)=>{assert.equal(ref,'component:payments/client');return entity;},
    service:{request:async(route,body)=>{sent={route,body};return {id:'job',request:body,state:'queued'};}}});
  const result=await fetch(`${base}/onboard`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({entityRef:'component:payments/client',source:{repositoryUrl:'https://evil.example/code'},consumesApis:['api:default/evil']})});
  assert.equal(result.status,202);
  assert.equal(sent.route,'/v1/onboard');
  assert.equal(sent.body.source.repositoryUrl,'https://github.com/acme/client.git');
  assert.deepEqual(sent.body.consumesApis,['api:payments/quotes']);
});
test('catalog denial prevents onboarding and job disclosure',async t=>{
  let submitted=false;
  const base=await host(t,{readEntity:async()=>{throw new HttpError(403,'ENTITY_UNAVAILABLE','denied');},
    service:{request:async(route)=>{if(route==='/v1/onboard')submitted=true;return {request:{entityRef:'component:default/secret'}};}}});
  const post=await fetch(`${base}/onboard`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({entityRef:'component:default/secret'})});
  assert.equal(post.status,403);assert.equal(submitted,false);
  const job=await fetch(`${base}/jobs/00000000-0000-0000-0000-000000000000`);
  assert.equal(job.status,403);assert(!(await job.text()).includes('component:default/secret'));
});
test('graph reads authorize the stored application identity before reading nodes',async t=>{
  const calls=[];
  const base=await host(t,{readEntity:async(_r,ref)=>{calls.push(ref);throw new HttpError(403,'ENTITY_UNAVAILABLE','denied');},
    service:{request:async(route)=>{calls.push(route);return {applicationKey:key,entityRef:'component:default/secret'};}}});
  const result=await fetch(`${base}/applications/${key}/nodes`);
  assert.equal(result.status,403);
  assert.deepEqual(calls,[`/v1/applications/${key}`,'component:default/secret']);
});
test('connections omit hidden providers and fail on catalog outages',async t=>{
  let outage=false;
  const connection={consumer:'component:default/client',provider:'component:default/provider',apiRef:'api:default/quotes'};
  const base=await host(t,{readEntity:async(_r,ref)=>{if(ref===connection.provider)throw new HttpError(outage?502:403,'ENTITY_UNAVAILABLE','unavailable');return {...entity,metadata:{name:ref.split('/')[1]},relations:[]};},
    service:{request:async(route)=>route==='/v1/applications'?{applications:[{entityRef:connection.consumer},{entityRef:connection.provider}]}:{connections:[connection]}}});
  let result=await fetch(`${base}/connections`);assert.equal(result.status,200);assert.deepEqual(await result.json(),{connections:[]});
  outage=true;result=await fetch(`${base}/connections`);assert.equal(result.status,502);
});
test('catalog relationship edits change connections without reindexing graphs',async t=>{
  let enabled=true;
  let indexed=0;
  const base=await host(t,{readEntity:async(_request,ref)=>({kind:ref.startsWith('api:')?'API':'Component',
    metadata:{name:ref.split('/')[1]},relations:ref.endsWith('/provider')?[{type:'providesApi',targetRef:'api:default/quotes'}]:
      enabled&&ref.endsWith('/client')?[{type:'consumesApi',targetRef:'api:default/quotes'}]:[]}),
    service:{request:async(route,body)=>{
      if(route==='/v1/applications')return {applications:[{entityRef:'component:default/client'},{entityRef:'component:default/provider'}]};
      if(route==='/v1/onboard')indexed++;
      assert.equal(route,'/v1/connections');
      return {connections:body.applications.some(a=>a.consumesApis.length)?[{consumer:'component:default/client',provider:'component:default/provider',apiRef:'api:default/quotes'}]:[]};
    }}});
  let result=await fetch(`${base}/connections`);assert.equal((await result.json()).connections.length,1);
  enabled=false;result=await fetch(`${base}/connections`);assert.equal((await result.json()).connections.length,0);
  assert.equal(indexed,0);
});
