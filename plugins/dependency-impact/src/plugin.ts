import {
  createComponentExtension,
  createPlugin,
  createRoutableExtension,
  createRouteRef,
  createApiFactory,
  discoveryApiRef,
  fetchApiRef,
} from "@backstage/core-plugin-api";
import { DependencyImpactClient, dependencyImpactApiRef } from "./api";

const rootRouteRef = createRouteRef({ id: "dependency-impact" });

export const dependencyImpactPlugin = createPlugin({
  id: "dependency-impact",
  routes: { root: rootRouteRef },
  apis: [
    createApiFactory({
      api: dependencyImpactApiRef,
      deps: { discovery: discoveryApiRef, fetchApi: fetchApiRef },
      factory: ({ discovery, fetchApi }) =>
        new DependencyImpactClient(discovery, fetchApi),
    }),
  ],
});

export const DependencyImpactPage = dependencyImpactPlugin.provide(
  createRoutableExtension({
    name: "DependencyImpactPage",
    component: () => import("./components").then((m) => m.DependencyImpactPage),
    mountPoint: rootRouteRef,
  })
);

export const EntityDependencyImpactContent = dependencyImpactPlugin.provide(
  createComponentExtension({
    name: "EntityDependencyImpactContent",
    component: {
      lazy: () =>
        import("./components").then((m) => m.EntityDependencyImpactContent),
    },
  })
);
