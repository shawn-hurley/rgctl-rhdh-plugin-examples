import {
  coreServices,
  createBackendPlugin,
} from "@backstage/backend-plugin-api";
import { Router, json } from "express";
import type { RequestHandler, ErrorRequestHandler } from "express";
import { AnalysisClient, AnalysisUnavailableError } from "./analysisClient";
import { DemoInputDiscoveryService } from "./inputDiscovery";
import type { InputDiscoveryService } from "./inputDiscovery";

export default createBackendPlugin({
  pluginId: "dependency-impact",
  register(env) {
    env.registerInit({
      deps: {
        httpRouter: coreServices.httpRouter,
        logger: coreServices.logger,
        config: coreServices.rootConfig,
        discovery: coreServices.discovery,
        auth: coreServices.auth,
        httpAuth: coreServices.httpAuth,
      },
      async init({ httpRouter, logger, config, discovery, auth, httpAuth }) {
        const router = Router();
        const inputDiscovery: InputDiscoveryService =
          new DemoInputDiscoveryService();
        router.use(json({ limit: "32kb" }));
        const baseUrl = config.getOptionalString(
          "dependencyImpact.service.baseUrl"
        );
        const serviceToken = config.getOptionalString(
          "dependencyImpact.service.token"
        );
        const analysis = new AnalysisClient(baseUrl, serviceToken);
        router.get("/status", async (_request, response) => {
          response.json({
            status: "ready",
            analysisAvailable: await analysis.isAvailable(),
          });
        });
        const authorizeEntity = async (
          request: Parameters<typeof httpAuth.credentials>[0],
          entityRef: unknown
        ) => {
          if (
            typeof entityRef !== "string" ||
            !/^component:[a-z0-9_.-]+\/[a-z0-9_.-]+$/.test(entityRef)
          ) {
            return 400;
          }
          const credentials = await httpAuth.credentials(request);
          const { token } = await auth.getPluginRequestToken({
            onBehalfOf: credentials,
            targetPluginId: "catalog",
          });
          const [kind, namespace, name] = entityRef.split(/[:/]/);
          const url = await discovery.getBaseUrl("catalog");
          // Use the caller's delegated credentials so catalog read authorization
          // also applies to submissions and revision-specific report/source reads.
          const result = await fetch(
            `${url}/entities/by-name/${kind}/${namespace}/${name}`,
            {
              headers: { Authorization: `Bearer ${token}` },
              signal: AbortSignal.timeout(15_000),
            }
          );
          return result.ok
            ? 200
            : result.status === 404
            ? 404
            : result.status === 403
            ? 403
            : 502;
        };
        router.get("/inputs", async (request, response, next) => {
          try {
            const entityRef = request.query.entityRef;
            if (entityRef !== undefined) {
              const allowed = await authorizeEntity(request, entityRef);
              if (allowed !== 200) {
                response
                  .status(allowed)
                  .json({
                    error: {
                      code: "ENTITY_UNAVAILABLE",
                      message:
                        "Catalog entity is invalid, unavailable, or not authorized",
                    },
                  });
                return;
              }
            }
            const result = await inputDiscovery.discoverInputs({
              entityRef: entityRef as string | undefined,
            });
            // Global discovery only lists catalog entities visible to this caller.
            // Catalog outages fail discovery instead of looking like no choices.
            if (entityRef === undefined) {
              const access = await Promise.all(
                result.inputs.map((input) =>
                  authorizeEntity(request, input.entityRef)
                )
              );
              if (
                access.some(
                  (status) => status !== 200 && status !== 403 && status !== 404
                )
              ) {
                response
                  .status(502)
                  .json({
                    error: {
                      code: "DISCOVERY_UNAVAILABLE",
                      message: "Unable to read available catalog applications",
                    },
                  });
                return;
              }
              result.inputs = result.inputs.filter(
                (_input, index) => access[index] === 200
              );
            }
            response.json(result);
          } catch (error) {
            next(error);
          }
        });
        router.post("/comparisons", async (request, response, next) => {
          try {
            const allowed = await authorizeEntity(
              request,
              request.body?.entityRef
            );
            if (allowed !== 200) {
              response
                .status(allowed)
                .json({
                  error: {
                    code: "ENTITY_UNAVAILABLE",
                    message:
                      "Catalog entity is invalid, unavailable, or not authorized",
                  },
                });
              return;
            }
            const result = await analysis.submit(request.body);
            const retry = result.retryAfter;
            if (retry) response.setHeader("Retry-After", retry);
            response.status(result.status).json(result.data);
          } catch (error) {
            next(error);
          }
        });
        const readComparison =
          (report: boolean): RequestHandler =>
          async (request, response, next) => {
            try {
              const id = request.params.id;
              if (typeof id !== "string" || !/^[0-9a-f-]{36}$/.test(id)) {
                response
                  .status(404)
                  .json({
                    error: {
                      code: "NOT_FOUND",
                      message: "Comparison not found",
                    },
                  });
                return;
              }
              const status = await analysis.getJob(id);
              const job = status.data;
              if (status.status !== 200 || !("request" in job)) {
                response.status(status.status).json(job);
                return;
              }
              const allowed = await authorizeEntity(
                request,
                job.request?.entityRef
              );
              if (allowed !== 200) {
                response
                  .status(allowed)
                  .json({
                    error: {
                      code: "ENTITY_UNAVAILABLE",
                      message:
                        "Catalog entity is unavailable or not authorized",
                    },
                  });
                return;
              }
              if (!report) {
                response.json(job);
                return;
              }
              const result = await analysis.getReport(id);
              response.status(result.status).json(result.data);
            } catch (error) {
              next(error);
            }
          };
        router.get("/comparisons/:id", readComparison(false));
        router.get("/comparisons/:id/report", readComparison(true));
        const serviceErrors: ErrorRequestHandler = (
          error,
          _request,
          response,
          next
        ) => {
          if (!(error instanceof AnalysisUnavailableError)) {
            next(error);
            return;
          }
          logger.warn(error.message);
          response
            .status(503)
            .json({
              error: { code: "ANALYSIS_UNAVAILABLE", message: error.message },
            });
        };
        router.use(serviceErrors);
        httpRouter.use(router);
        logger.info("Dependency impact backend loaded");
      },
    });
  },
});
