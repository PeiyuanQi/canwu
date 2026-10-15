# canwu-military

Reusable military domain contracts and simulation plugin for Canwu. The
extension owns military records and rules while resource, transport, society,
law, fiscal, knowledge, and information providers retain ownership of their
state. It is intentionally period-neutral; reference content and integrations
provide unit types, doctrine, terrain, and scenario data.

Every operation-keyed random draw the plugin makes (reconnaissance, special
operations, combat rounds) cites as its operation evidence the admitted packet
that carries the command or plugin tick being applied. That packet exists in
every run, so a draw is accepted alike in a live run, a sealed run, and exact
replay. A command whose apply step fails is recorded as a rejected outcome with
a `canwu.military.ingress_rejected.v1` event; a plugin tick that fails is a
broken plugin invariant and still fails the boundary.
