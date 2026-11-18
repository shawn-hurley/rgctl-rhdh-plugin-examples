import type { ComparisonInput } from "./inputDiscovery";

/** Controlled demo discovery manifest; contains no analysis results or source code. */
export const demoInputs: readonly ComparisonInput[] = [
  {
    id: "checkout",
    label: "Checkout — internal helper change",
    description:
      "An internal price-normalization change reaches application code through an unchanged public API.",
    packageManager: "maven",
    entityRef: "component:default/dependency-impact-checkout",
    applicationRevision: "demo-baseline",
    dependency: {
      groupId: "io.rgctl.demo",
      artifactId: "pricing",
      type: "jar",
      classifier: null,
    },
    baselineVersion: "1.0.0",
    targetVersion: "1.1.0",
    strategy: "direct-upgrade",
  },
  {
    id: "delivery",
    label: "Delivery — public function change",
    description:
      "A changed public shipping quote function affects its application callers.",
    packageManager: "maven",
    entityRef: "component:default/dependency-impact-delivery",
    applicationRevision: "demo-baseline",
    dependency: {
      groupId: "io.rgctl.demo",
      artifactId: "shipping-rules",
      type: "jar",
      classifier: null,
    },
    baselineVersion: "1.0.0",
    targetVersion: "1.1.0",
    strategy: "direct-upgrade",
  },
  {
    id: "notification",
    label: "Notification — transitive override",
    description:
      "Override token-core while template-api stays at 1.0.0, modeling CVE remediation without a direct-dependency release.",
    packageManager: "maven",
    entityRef: "component:default/dependency-impact-notification",
    applicationRevision: "demo-baseline",
    dependency: {
      groupId: "io.rgctl.demo",
      artifactId: "token-core",
      type: "jar",
      classifier: null,
    },
    baselineVersion: "1.0.0",
    targetVersion: "1.0.1",
    strategy: "transitive-override",
  },
  {
    id: "file-import",
    label: "File import — released patch upgrade",
    description:
      "A Commons IO patch changes file-comparison behavior while the public signature remains unchanged.",
    packageManager: "maven",
    entityRef: "component:default/dependency-impact-file-import",
    applicationRevision: "demo-baseline",
    dependency: {
      groupId: "commons-io",
      artifactId: "commons-io",
      type: "jar",
      classifier: null,
    },
    baselineVersion: "2.15.0",
    targetVersion: "2.15.1",
    strategy: "direct-upgrade",
  },
  {
    id: "no-impact",
    label: "Pricing maintenance — no application impact",
    description:
      "A patch changes an unused library function while application-used price calculation remains unchanged.",
    packageManager: "maven",
    entityRef: "component:default/dependency-impact-no-impact",
    applicationRevision: "demo-baseline",
    dependency: {
      groupId: "io.rgctl.demo",
      artifactId: "pricing-maintenance",
      type: "jar",
      classifier: null,
    },
    baselineVersion: "1.0.0",
    targetVersion: "1.0.1",
    strategy: "direct-upgrade",
  },
  {
    "id": "injected-policy",
    "label": "Constructor injection — selected policy",
    "description": "Trace an updated risk policy through an injected interface into order review.",
    "packageManager": "maven",
    "entityRef": "component:default/dependency-impact-injected-policy",
    "applicationRevision": "demo-baseline",
    "dependency": {
      "groupId": "io.rgctl.demo",
      "artifactId": "injected-risk-policy",
      "type": "jar",
      "classifier": null
    },
    "baselineVersion": "1.0.0",
    "targetVersion": "1.0.1",
    "strategy": "direct-upgrade"
  },
  {
    "id": "caller-separation",
    "label": "Caller separation — submission and preview",
    "description": "Compare submission and preview callers that pass different policies through the same gateway.",
    "packageManager": "maven",
    "entityRef": "component:default/dependency-impact-caller-separation",
    "applicationRevision": "demo-baseline",
    "dependency": {
      "groupId": "io.rgctl.demo",
      "artifactId": "routing-risk-policy",
      "type": "jar",
      "classifier": null
    },
    "baselineVersion": "1.0.0",
    "targetVersion": "1.0.1",
    "strategy": "direct-upgrade"
  },
  {
    "id": "stable-preview",
    "label": "Stable preview — unchanged implementation",
    "description": "A changed strict policy shares an interface with the basic policy selected through static storage and a lambda.",
    "packageManager": "maven",
    "entityRef": "component:default/dependency-impact-stable-preview",
    "applicationRevision": "demo-baseline",
    "dependency": {
      "groupId": "io.rgctl.demo",
      "artifactId": "preview-risk-policy",
      "type": "jar",
      "classifier": null
    },
    "baselineVersion": "1.0.0",
    "targetVersion": "1.0.1",
    "strategy": "direct-upgrade"
  },
  {
  "id": "go-public",
  "label": "Go checkout \u2014 public function update",
  "description": "A public Quote change reaches checkout code.",
  "packageManager": "gomod",
  "entityRef": "component:default/dependency-impact-go-public",
  "applicationRevision": "demo-baseline",
  "dependency": {
    "modulePath": "example.test/rgctl/public"
  },
  "baselineVersion": "v1.0.0",
  "targetVersion": "v1.0.1",
  "strategy": "direct-upgrade"
},
  {
  "id": "go-patch",
  "label": "Go validation \u2014 patch release",
  "description": "A v1.0.1 validation fix changes behavior without changing its signature.",
  "packageManager": "gomod",
  "entityRef": "component:default/dependency-impact-go-patch",
  "applicationRevision": "demo-baseline",
  "dependency": {
    "modulePath": "example.test/rgctl/patch"
  },
  "baselineVersion": "v1.0.0",
  "targetVersion": "v1.0.1",
  "strategy": "direct-upgrade"
},
  {
  "id": "go-internal",
  "label": "Go checkout \u2014 internal helper update",
  "description": "An unexported normalize change reaches Run through the unchanged public Quote function.",
  "packageManager": "gomod",
  "entityRef": "component:default/dependency-impact-go-internal",
  "applicationRevision": "demo-baseline",
  "dependency": {
    "modulePath": "example.test/rgctl/internal"
  },
  "baselineVersion": "v1.0.0",
  "targetVersion": "v1.0.1",
  "strategy": "direct-upgrade"
},
  {
  "id": "go-no-impact",
  "label": "Go pricing \u2014 unaffected update",
  "description": "Only UnusedPreview changes; the application calls the unchanged Quote path.",
  "packageManager": "gomod",
  "entityRef": "component:default/dependency-impact-go-no-impact",
  "applicationRevision": "demo-baseline",
  "dependency": {
    "modulePath": "example.test/rgctl/no-impact"
  },
  "baselineVersion": "v1.0.0",
  "targetVersion": "v1.0.1",
  "strategy": "direct-upgrade"
},
  {
  "id": "go-transitive",
  "label": "Go notification \u2014 transitive override",
  "description": "Bump token-core to v1.0.1 while template stays at v1.0.0; the unchanged wrapper carries the impact to Run.",
  "packageManager": "gomod",
  "entityRef": "component:default/dependency-impact-go-transitive",
  "applicationRevision": "demo-baseline",
  "dependency": {
    "modulePath": "example.test/rgctl/token-core"
  },
  "baselineVersion": "v1.0.0",
  "targetVersion": "v1.0.1",
  "strategy": "transitive-override"
},
  {
  "id": "go-injected",
  "label": "Go checkout \u2014 injected interface implementation",
  "description": "A changed RiskPolicy implementation is injected as Scorer and used by Checkout.Submit.",
  "packageManager": "gomod",
  "entityRef": "component:default/dependency-impact-go-injected",
  "applicationRevision": "demo-baseline",
  "dependency": {
    "modulePath": "example.test/rgctl/risk"
  },
  "baselineVersion": "v1.0.0",
  "targetVersion": "v1.0.1",
  "strategy": "direct-upgrade"
},
];
