const {test}=require('node:test');const assert=require('node:assert/strict');const path=require('node:path');
const Module=require('node:module');const original=Module._load;
const {onboardingRequest}=require(path.resolve('local-dev/.generated/application-graph-tests/catalog.js'));
Module._load=function(name,...args){if(name==='@internal/backstage-plugin-application-graph-backend')return {onboardingRequest};return original.call(this,name,...args);};
const {ApplicationGraphProcessor}=require(path.resolve('local-dev/.generated/catalog-import-tests/processor.js'));Module._load=original;
const entity=()=>({apiVersion:'backstage.io/v1alpha1',kind:'Component',metadata:{name:'app',annotations:{'application-graph.io/enabled':'true','backstage.io/source-location':'url:https://github.com/acme/app'}},spec:{type:'service',lifecycle:'experimental',owner:'team'}});
const location={type:'url',target:'https://example.org/catalog-info.yaml'};
test('import queues once and subsequent processing enriches a completed graph',async()=>{
 const input=entity();let submits=0;let state='queued';const emitted=[];
 const processor=new ApplicationGraphProcessor({warn:()=>{},readEntities:async()=>[input],service:{request:async(path,body)=>{
   if(path==='/v1/onboard'){submits++;return {id:'job',state};}
   if(path==='/v1/jobs/job')return {state};
   if(path==='/v1/applications')return {applications:[{entityRef:'component:default/app',graphVersion:'version',source:onboardingRequest(input,'component:default/app').source,operations:[{direction:'provides',path:'/hello',methods:['GET']}]}]};
   throw Error(path);
 }}});
 const queued=await processor.preProcessEntity(input,location,e=>emitted.push(e));assert.equal(queued.metadata.annotations['application-graph.io/status'],'queued');assert.equal(queued.spec.providesApis,undefined);
 state='succeeded';const indexed=await processor.preProcessEntity(input,location,e=>emitted.push(e));
 assert.equal(submits,1);assert.equal(indexed.metadata.annotations['application-graph.io/status'],'indexed');assert.deepEqual(indexed.spec.providesApis,['api:default/app-api']);assert.equal(emitted.length,1);assert.equal(emitted[0].entity.kind,'API');
});
test('ordinary imports do not trigger analysis; service failure preserves descriptor',async()=>{
 let requests=0;const input=entity();delete input.metadata.annotations['application-graph.io/enabled'];
 const processor=new ApplicationGraphProcessor({warn:()=>{},readEntities:async()=>[],service:{request:async()=>{requests++;throw Error('offline');}}});
 assert.equal(await processor.preProcessEntity(input,location,()=>{}),input);assert.equal(requests,0);
 input.metadata.annotations['application-graph.io/enabled']='true';input.spec.dependsOn=['resource:default/manual'];
 const failed=await processor.preProcessEntity(input,location,()=>{});assert.equal(failed.metadata.annotations['application-graph.io/status'],'failed');assert.deepEqual(failed.spec.dependsOn,input.spec.dependsOn);
});
