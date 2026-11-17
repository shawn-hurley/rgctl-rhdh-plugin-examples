export interface Entity {
  kind: string;
  metadata: {name: string; namespace?: string; annotations?: Record<string,string>};
  spec?: {providesApis?: string[]; consumesApis?: string[]; dependsOn?: string[]; dependencyOf?: string[]; subcomponentOf?: string};
  relations?: {type: string; targetRef: string}[];
}
export class HttpError extends Error {
  constructor(readonly status: number, readonly code: string, message: string) { super(message); }
}
export function entityRef(value: unknown, kind = 'component', namespace = 'default'): string {
  if (typeof value !== 'string' || value.length > 200) throw new HttpError(400,'INVALID_ENTITY','Expected an entity reference');
  const typed = value.includes(':') ? value.split(':') : [kind,value];
  if (typed.length !== 2) throw new HttpError(400,'INVALID_ENTITY','Invalid entity reference');
  const scoped = typed[1].includes('/') ? typed[1].split('/') : [namespace,typed[1]];
  if (scoped.length !== 2 || ![typed[0],...scoped].every(x => /^[a-z0-9](?:[a-z0-9_.-]*[a-z0-9])?$/i.test(x) && x.length <= 63)) throw new HttpError(400,'INVALID_ENTITY','Invalid entity reference');
  if (typed[0].toLowerCase() !== kind) throw new HttpError(400,'INVALID_ENTITY',`Expected ${kind}`);
  return `${kind}:${scoped[0]}/${scoped[1]}`.toLowerCase();
}
export function source(location: string | undefined) {
  if (!location || (!location.startsWith('url:') && !location.startsWith('file:'))) throw new HttpError(400,'SOURCE_LOCATION_REQUIRED','Set backstage.io/source-location to the application repository URL');
  let url: URL;
  try { url = new URL(location.startsWith('url:')?location.slice(4):location); } catch { throw new HttpError(400,'INVALID_SOURCE','Invalid source URL'); }
  if (url.protocol === 'file:' && !url.username && !url.password && !url.search && !url.hash && !url.hostname) return {repositoryUrl:url.toString(),revision:'HEAD',subPath:''};
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash || url.port) throw new HttpError(400,'INVALID_SOURCE','Use an HTTPS source URL without credentials or options');
  const parts = url.pathname.split('/').filter(Boolean);
  let revision = 'HEAD';
  let subPath = '';
  let tree = -1;
  if (url.hostname === 'github.com' && parts[2] === 'tree') tree = 2;
  const gitlab = parts.indexOf('-');
  if (gitlab >= 0 && parts[gitlab+1] === 'tree') tree = gitlab;
  if (tree >= 0) {
    const offset = parts[tree] === '-' ? 2 : 1;
    revision = decodeURIComponent(parts[tree+offset] ?? '');
    subPath = parts.slice(tree+offset+1).map(decodeURIComponent).join('/');
    url.pathname = parts.slice(0,tree).join('/');
  }
  if (!revision || !url.pathname.replace(/\//g,'')) throw new HttpError(400,'INVALID_SOURCE','A repository and revision are required');
  const repositoryUrl = url.toString().replace(/\/$/,'').replace(/\.git$/,'')+'.git';
  return {repositoryUrl, revision, subPath};
}
export function catalogRelations(entity: Entity, requested: string) {
  const namespace = entity.metadata.namespace ?? 'default';
  const canonical = entityRef(`${entity.kind}:${namespace}/${entity.metadata.name}`);
  if (canonical !== requested) throw new HttpError(502,'CATALOG_IDENTITY_MISMATCH','Catalog returned a different application');
  const apis = (type: 'provides' | 'consumes') => {
    // Processed catalog relations are authoritative. Raw entity descriptors may
    // omit relations, in which case standard spec fields supply the references.
    const refs = entity.relations === undefined
      ? entity.spec?.[type === 'provides' ? 'providesApis' : 'consumesApis'] ?? []
      : entity.relations.filter(r => r.type === `${type}Api`).map(r => r.targetRef);
    return [...new Set(refs.map(r => entityRef(r,'api',namespace)))];
  };
  return {entityRef: canonical, providesApis: apis('provides'), consumesApis: apis('consumes')};
}
export function onboardingRequest(entity: Entity, requested: string) {
  const selected = source(entity.metadata.annotations?.['backstage.io/source-location']);
  return {...catalogRelations(entity, requested), source: {...selected,
    ...(entity.metadata.annotations?.['application-graph.io/maven-sources']==='true'?{includeMavenDependencies:true}:{})}};
}
