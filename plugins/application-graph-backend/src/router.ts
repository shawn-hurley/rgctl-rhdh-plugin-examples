import { Router, json, Request, Response, NextFunction } from 'express';
import { Entity, HttpError, entityRef, onboardingRequest, catalogRelations } from './catalog';
import { GraphClient, Application, Connection, Job } from './client';
import {catalogRef,relationships,GraphEvidence} from './relationships';

export function createRouter(options: {service: GraphClient; readEntity: (request: Request, ref: string) => Promise<Entity>}) {
  const router = Router();
  router.use(json({limit:'32kb'}));
  const route = (fn: (request: Request, response: Response) => Promise<void>) => (request: Request,response: Response,next: NextFunction) => { fn(request,response).catch(next); };
  const app = async (request: Request) => {
    const key = request.params.key;
    if (typeof key !== 'string' || !/^[a-f0-9]{64}$/.test(key)) throw new HttpError(404,'NOT_FOUND','Application not found');
    const data = await options.service.request<Application>(`/v1/applications/${key}`);
    await options.readEntity(request,entityRef(data.entityRef));
    return data;
  };
  router.get('/status',route(async (_request,response) => {
    let ready = false;
    try { await options.service.request('/health'); ready = true; } catch { /* unavailable */ }
    response.json({status:'ready',graphServiceAvailable:ready});
  }));
  router.post('/onboard',route(async (request,response) => {
    const ref = entityRef(request.body?.entityRef);
    const entity = await options.readEntity(request,ref);
    // Source locations and relationships come from the authorized catalog entity,
    // never from request-body overrides or hardcoded application manifests.
    const job = await options.service.request<Job>('/v1/onboard',onboardingRequest(entity,ref));
    response.status(202).json(job);
  }));
  router.get('/jobs/:id',route(async (request,response) => {
    const id = request.params.id;
    if (typeof id !== 'string' || !/^[0-9a-f-]{36}$/.test(id)) throw new HttpError(404,'NOT_FOUND','Job not found');
    const job = await options.service.request<Job>(`/v1/jobs/${id}`);
    await options.readEntity(request,entityRef(job.request.entityRef));
    response.json(job);
  }));
  router.get('/applications',route(async (request,response) => {
    const data = await options.service.request<{applications:Application[]}>('/v1/applications');
    const visible: Application[] = [];
    for (const application of data.applications) {
      try { await options.readEntity(request,entityRef(application.entityRef)); visible.push(application); }
      catch (error) { if (!(error instanceof HttpError) || ![403,404].includes(error.status)) throw error; }
    }
    response.json({applications:visible});
  }));
  router.get('/applications/:key',route(async (request,response) => { response.json(await app(request)); }));
  router.get('/applications/:key/versions/:version',route(async (request,response) => {
    const application = await app(request);
    const version = request.params.version;
    if (typeof version !== 'string' || !/^[0-9a-f-]{36}$/.test(version)) throw new HttpError(404,'NOT_FOUND','Graph version not found');
    response.json(await options.service.request(`/v1/applications/${application.applicationKey}/versions/${version}`));
  }));
  for (const collection of ['nodes','edges']) {
    router.get(`/applications/:key/${collection}`,route(async (request,response) => {
      const application = await app(request);
      const query = new URLSearchParams();
      for (const [key,value] of Object.entries(request.query)) {
        if (!['offset','limit','nodeId','graphVersion'].includes(key) || typeof value !== 'string') throw new HttpError(400,'INVALID_QUERY','Invalid graph query');
        if (['offset','limit'].includes(key) && (!/^\d+$/.test(value) || !Number.isSafeInteger(Number(value)) || (key === 'limit' && (Number(value)<1 || Number(value)>1000)))) throw new HttpError(400,'INVALID_QUERY','Invalid graph page');
        if (['nodeId','graphVersion'].includes(key) && !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value)) throw new HttpError(400,'INVALID_QUERY','Invalid graph identity');
        query.set(key,value);
      }
      response.json(await options.service.request(`/v1/applications/${application.applicationKey}/${collection}?${query}`));
    }));
  }
  router.get('/connections',route(async (request,response) => {
    const filter = request.query.entityRef === undefined ? undefined : entityRef(request.query.entityRef);
    if (filter) await options.readEntity(request,filter);
    const applications = await options.service.request<{applications:Application[]}>('/v1/applications');
    const currentRelations: ReturnType<typeof catalogRelations>[] = [];
    for (const application of applications.applications) {
      try {
        const ref = entityRef(application.entityRef);
        currentRelations.push(catalogRelations(await options.readEntity(request,ref),ref));
      } catch (error) { if (!(error instanceof HttpError) || ![403,404].includes(error.status)) throw error; }
    }
    const data = await options.service.request<{connections:Connection[]}>('/v1/connections',{applications:currentRelations});
    const connections: Connection[] = [];
    const checked = new Set<string>();
    for (const connection of data.connections) {
      if (filter && connection.consumer !== filter && connection.provider !== filter) continue;
      try {
        for (const ref of [entityRef(connection.consumer),entityRef(connection.provider),entityRef(connection.apiRef,'api')]) {
          if (!checked.has(ref)) { await options.readEntity(request,ref); checked.add(ref); }
        }
        connections.push(connection);
      } catch (error) { if (!(error instanceof HttpError) || ![403,404].includes(error.status)) throw error; }
    }
    response.json({connections});
  }));
  router.get('/relationships',route(async(request,response)=>{
    const ref=request.query.entityRef===undefined?undefined:catalogRef(request.query.entityRef);
    const depth=request.query.depth===undefined?4:Number(request.query.depth);
    if(!Number.isInteger(depth)||depth<1||depth>8)throw new HttpError(400,'INVALID_DEPTH','Depth must be between 1 and 8');
    const data=await options.service.request<{applications:GraphEvidence[]}>('/v1/applications');
    response.json(await relationships({request,readEntity:options.readEntity,applications:data.applications,entity:ref,depth}));
  }));
  router.use((error: unknown,_request: Request,response: Response,next: NextFunction) => {
    if (!(error instanceof HttpError)) { next(error); return; }
    response.status(error.status).json({error:{code:error.code,message:error.message}});
  });
  return router;
}
