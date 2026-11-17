# Remaining possible-origin models

Authorized follow-up to the `fe17294d` qualification: 117/169 impacts detected,
52 missed. Preserve recall; additional warnings are acceptable. Work remains
native in rgctl, with the adapter supplying canonical graph candidates.

1. **Expression transfers.** Start with the failing conditional-factory and
   assertion fixtures. End with selected alternatives and successful assertions
   retaining origins, without treating conditions or arithmetic as aliases.
   Implemented in signed commit `51ceaff0`; both conditional-factory demos and
   Go type assertion recover in targeted runs. Go type-switch binding is supplied by `6b2c0f66`.
2. **State.** Start with explicit store/load facts and constructor receiver
   binding. End with supported fields retaining possible stored origins across
   methods, bounded and separately qualified. Native slice committed as
   `95e1273a`: explicit fields, TypeScript parameter properties, Go keyed structs.
   Adapter constructor integration is implemented locally. Implicit Java fields,
   module initialization, generated dataclass constructors and descriptors remain.
3. **Aggregate boundaries.** Start with distinct container identities and
   element values. End with indexed elements, variadic packing and return-slot
   projections contributing possible origins, with conservative alternatives.
   Collections/variadic transfers are committed as `3675983a`; Go return projections and type-switch aliases are committed as `6b2c0f66`.
4. **Callable and deferred values.** Start with native callable identities and
   bound receiver facts. End with supported callback invocation and deferred
   handoffs adding canonical calls; unsupported scheduling/captures stay explicit.
5. **Dynamic lookup.** Start with statically known lookup facts and compatible
   receiver candidates. End with supported property/registry lookups retaining
   all possible targets, and explicit unknown coverage for unsupported forms.

Each slice needs native regressions and required preflight, with a separate
signed commit. Run the full 254-case catalog and 19 Spring cases on the final
clean native build; compare each watched function and unchanged runtime input
fingerprints against `fe17294d`. Preserve all historical evidence. No absence of
warnings certifies safety while relevant coverage is partial. No CI, pinned
caches/toolchains, external semantic tools or deployment changes are required.

Latest full qualification at `3675983a`: **135/169 detected**, 18 recovered, no lost
positives, 34 remaining misses, zero execution failures. Spring and inputs unchanged.

## Final implementation and qualification

Callable/bound receiver values are implemented as `2973e14c`; explicit deferred
handoffs as `d2ebf9db`; computed/getattr lookup, constructor registries and
prototype construction as `4db0d68a`. These implement bounded forms in each
priority group; the unsupported forms identified above remain open.

The [final report](../demo-evaluation/value-flow/REMAINING-MODELS-QUALIFICATION.md)
records **155/169 detected**, 38 recovered from the starting `fe17294d` baseline,
no lost detections, 14 remaining misses, zero execution failures and 46/90 negative
warnings. All fixture inputs and Spring classifications are unchanged. Required
preflight and native/adapter regressions pass. No deployment or publication was
performed. Further work is explicitly listed in the report; upgrade safety and
complete feature coverage are not qualified.

## Environment follow-up

User-directed investigation identified a common external binding layer. Signed
commit `c87e8666` implements native module/lexical/captured environment transfers,
source imports and same-package state, literal callable identities, implicit Java
members, Java member roles and generic constructor heads. The [environment report](../demo-evaluation/value-flow/ENVIRONMENT-QUALIFICATION.md)
records 162/169 impacts, seven recovered, no lost detections, seven remaining
misses and zero execution failures. All 58 observations from 29 scope controls
match expected runtimes. Missing implicit/generated/library invocation effects
and Go anonymous graph identity remain explicit; shared taint and upgrade-safety
qualification are still incomplete. No live deployment or GitHub publication.
