import {createHash} from 'node:crypto';
import type {Entity} from '@backstage/catalog-model';

export interface Manifest {
  entityRef:string;graphVersion:string;source:unknown;projectCoordinate?:string;
  artifacts?:{coordinate:string;direct:boolean}[];
  resourceUses?:{uri?:string;applicationPaths:unknown[]}[];
  operations:{direction:string;methods:string[];path?:string;url?:string}[];
}
export function ref(entity:Entity) {
  return `${entity.kind}:${entity.metadata.namespace??'default'}/${entity.metadata.name}`.toLowerCase();
}
export function apiRef(entity:Entity) {
  const name=entity.metadata.name.length<=59?`${entity.metadata.name}-api`:
    `${entity.metadata.name.slice(0,50)}-${createHash('sha256').update(ref(entity)).digest('hex').slice(0,8)}-api`;
  return `api:${entity.metadata.namespace??'default'}/${name}`.toLowerCase();
}
function routeMatches(route:string,path:string) {
  const a=route.replace(/^\/+|\/+$/g,'').split('/'),b=path.replace(/^\/+|\/+$/g,'').split('/');
  return a.length===b.length&&a.every((part,i)=>part===b[i]||/^\{[^/]+\}$/.test(part));
}
function serves(consumer:Entity,provider:Entity,manifest:Manifest,call:Manifest['operations'][number]) {
  const base=provider.metadata.annotations?.['application-graph.io/api-base-url'];
  if(!base||!call.url)return false;
  let host:URL,url:URL;
  try {
    host=new URL(base);
    const clientBase=consumer.metadata.annotations?.['application-graph.io/http-client-base-url'];
    url=clientBase?new URL(call.url,clientBase):new URL(call.url);
  }catch{return false;}
  if(!['http:','https:'].includes(host.protocol)||host.origin!==url.origin||host.username||host.password)return false;
  const prefix=host.pathname.replace(/\/$/,'');
  return manifest.operations.some(endpoint=>endpoint.direction==='provides'&&endpoint.path&&
    routeMatches(`${prefix}/${endpoint.path.replace(/^\//,'')}`,url.pathname)&&endpoint.methods.some(method=>call.methods.includes(method)));
}
/** Add graph-derived facts to each freshly read descriptor; never rewrite Git. */
export function enrich(entity:Entity,own:Manifest,manifests:Manifest[],entities:Entity[]) {
  const byRef=new Map(entities.map(e=>[ref(e),e]));
  const depends=new Set<string>();const provides=new Set<string>();const consumes=new Set<string>();
  for(const other of manifests) {
    if(other.entityRef===ref(entity)||!byRef.has(other.entityRef))continue;
    if(other.projectCoordinate&&own.artifacts?.some(a=>a.direct&&a.coordinate===other.projectCoordinate))depends.add(other.entityRef);
    const provider=byRef.get(other.entityRef)!;
    if(provider.metadata.annotations?.['application-graph.io/enabled']==='true'&&
      own.operations.some(call=>call.direction==='consumes'&&serves(entity,provider,other,call))) {
      consumes.add(apiRef(provider));depends.add(other.entityRef);
    }
  }
  for(const resource of entities.filter(e=>e.kind.toLowerCase()==='resource')) {
    const uri=resource.metadata.annotations?.['application-graph.io/resource-uri'];
    if(uri&&own.resourceUses?.some(use=>use.uri===uri&&use.applicationPaths.length))depends.add(ref(resource));
  }
  const endpoints=own.operations.filter(o=>o.direction==='provides'&&o.path&&o.methods.length);
  let api:Entity|undefined;
  if(endpoints.length) {
    const reference=apiRef(entity);provides.add(reference);
    const paths:Record<string,Record<string,unknown>>={};
    for(const endpoint of endpoints)for(const method of endpoint.methods) {
      if(!['GET','POST','PUT','PATCH','DELETE','HEAD','OPTIONS','TRACE'].includes(method))continue;
      (paths[endpoint.path!]??={})[method.toLowerCase()]={responses:{'200':{description:'Response'}}};
    }
    api={apiVersion:'backstage.io/v1alpha1',kind:'API',metadata:{name:reference.split('/')[1],namespace:entity.metadata.namespace??'default',
      annotations:{'application-graph.io/provider':ref(entity),'application-graph.io/graph-version':own.graphVersion}},
      spec:{type:'openapi',lifecycle:entity.spec?.lifecycle??'experimental',owner:entity.spec?.owner??'unknown',
        definition:JSON.stringify({openapi:'3.0.3',info:{title:entity.metadata.name,version:own.graphVersion},paths})}};
  }
  const spec={...entity.spec};
  for(const [key,derived] of [['dependsOn',depends],['providesApis',provides],['consumesApis',consumes]] as const) {
    const declared=Array.isArray(spec[key])?spec[key] as string[]:[];
    if(declared.length||derived.size)spec[key]=[...new Set([...declared,...derived])].sort();
  }
  return {entity:{...entity,spec},api};
}
