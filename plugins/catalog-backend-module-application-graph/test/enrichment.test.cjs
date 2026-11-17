const {test}=require('node:test');const assert=require('node:assert/strict');const path=require('node:path');
const {enrich,apiRef}=require(path.resolve('local-dev/.generated/catalog-import-tests/enrichment.js'));
const c=(name,annotations={},spec={})=>({apiVersion:'backstage.io/v1alpha1',kind:'Component',metadata:{name,annotations:{'application-graph.io/enabled':'true',...annotations}},spec:{owner:'team',lifecycle:'experimental',...spec}});
const manifest=(name,extra={})=>({entityRef:'component:default/'+name,source:{},graphVersion:'v1',operations:[],...extra});
const endpoint={direction:'provides',methods:['GET'],path:'/api/{id}'};
test('direct Maven dependencies and reachable resources populate standard fields',()=>{
 const entity=c('app');const library=c('library');const transitive=c('transitive');
 const resources=['used','unused'].map(name=>({kind:'Resource',metadata:{name,annotations:{'application-graph.io/resource-uri':'file:'+name}}}));
 const own=manifest('app',{artifacts:[{coordinate:'g:library:1',direct:true},{coordinate:'g:transitive:1',direct:false}],resourceUses:[{uri:'file:used',applicationPaths:[{}]},{uri:'file:unused',applicationPaths:[]}]});
 const result=enrich(entity,own,[own,manifest('library',{projectCoordinate:'g:library:1'}),manifest('transitive',{projectCoordinate:'g:transitive:1'})],[entity,library,transitive,...resources]);
 assert.deepEqual(result.entity.spec.dependsOn,['component:default/library','resource:default/used']);assert.equal(entity.spec.dependsOn,undefined);
});
test('provider generates a real OpenAPI entity and providesApis',()=>{
 const provider=c('provider');const result=enrich(provider,manifest('provider',{operations:[endpoint]}),[],[]);
 assert.deepEqual(result.entity.spec.providesApis,['api:default/provider-api']);assert.equal(result.api.kind,'API');
 assert(JSON.parse(result.api.spec.definition).paths['/api/{id}'].get);assert.equal(result.api.spec.owner,'team');
});
test('API matching requires origin, method and segment-compatible path',()=>{
 const consumer=c('consumer'),provider=c('provider',{'application-graph.io/api-base-url':'http://provider:8080'});
 const other=manifest('provider',{operations:[endpoint]});
 for(const [url,method,match] of [['http://provider:8080/api/42','GET',true],['http://unrelated:8080/api/42','GET',false],['http://provider:8080/api/42','POST',false],['http://provider:8080/api/42/nested','GET',false]]){
   const result=enrich(consumer,manifest('consumer',{operations:[{direction:'consumes',methods:[method],url}]}),[other],[provider]);
   assert.deepEqual(result.entity.spec.consumesApis,match?['api:default/provider-api']:undefined,url);
   assert.deepEqual(result.entity.spec.dependsOn,match?['component:default/provider']:undefined);
 }
});
test('declared relationships survive; absent providers are never fabricated',()=>{
 const entity=c('app',{}, {dependsOn:['resource:default/manual'],consumesApis:['manual-api'],subcomponentOf:'parent'});
 const result=enrich(entity,manifest('app',{artifacts:[{coordinate:'g:missing:1',direct:true}]}),[manifest('missing',{projectCoordinate:'g:missing:1'})],[]);
 assert.deepEqual(result.entity.spec.dependsOn,['resource:default/manual']);assert.equal(result.entity.spec.subcomponentOf,'parent');assert.deepEqual(result.entity.spec.consumesApis,['manual-api']);
});
test('long provider names produce stable valid API identities',()=>{
 const entity=c('a'.repeat(63));const reference=apiRef(entity);assert.equal(reference,apiRef(entity));assert(reference.split('/')[1].length<=63);
});
test('relative request paths require an explicitly supplied client base URL',()=>{
 const provider=c('provider',{'application-graph.io/api-base-url':'http://provider:8080'});
 const own=manifest('client',{operations:[{direction:'consumes',methods:['GET'],url:'/api/42'}]});
 const providerGraph=manifest('provider',{operations:[endpoint]});
 assert.equal(enrich(c('client'),own,[providerGraph],[provider]).entity.spec.consumesApis,undefined);
 assert.deepEqual(enrich(c('client',{'application-graph.io/http-client-base-url':'http://provider:8080'}),own,[providerGraph],[provider]).entity.spec.consumesApis,['api:default/provider-api']);
});
test('a provider that has not opted in cannot cause a dangling generated API',()=>{
 const provider=c('provider',{'application-graph.io/api-base-url':'http://provider:8080'});delete provider.metadata.annotations['application-graph.io/enabled'];
 const own=manifest('client',{operations:[{direction:'consumes',methods:['GET'],url:'http://provider:8080/api/42'}]});
 const result=enrich(c('client'),own,[manifest('provider',{operations:[endpoint]})],[provider]);assert.equal(result.entity.spec.consumesApis,undefined);
});
