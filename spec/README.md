# Specification

The durable design of Panic Button. Read in order before changing code. Requirements carry IDs (for example `FR-ACT-002`), which code and tests reference for traceability.

| File | Contents |
|---|---|
| [00-overview.md](00-overview.md) | Goals, scope, roles, glossary |
| [01-requirements.md](01-requirements.md) | Functional and non-functional requirements with IDs |
| [02-architecture.md](02-architecture.md) | Components, crates, process model |
| [03-protocol.md](03-protocol.md) | Wire protocol, delivery algorithm, timers (normative) |
| [04-security.md](04-security.md) | Threat model, keys, enrolment, revocation |
| [05-desktop-agent.md](05-desktop-agent.md) | Desktop UX and platform behaviour |
| [06-relay.md](06-relay.md) | Optional relay server |
| [07-testing.md](07-testing.md) | Simulator, properties, integration and hardware lab tests |
| [rule-coverage.md](rule-coverage.md) | Each rule in 03 and 04, and the tests that check it |
| [08-deployment.md](08-deployment.md) | MSI, policies, config files, relay hosting |
| [09-roadmap.md](09-roadmap.md) | v1 milestones (`R<n>` items with status) and later phases |
| [adr/](adr/) | Architecture decision records |
| [queries.md](queries.md) | Open questions that need a human decision |

The glossary in `00-overview.md` is the ubiquitous language. Use its terms in code identifiers, UI strings, CLI commands and tests.

The clinical safety hazard log lives in [../safety/hazard-log.md](../safety/hazard-log.md), linked from the root [SAFETY.md](../SAFETY.md).

If code intentionally diverges from the spec, update the spec in the same change, or record the question in `queries.md`.
