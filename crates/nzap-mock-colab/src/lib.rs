//! Mock Google services for NZAP Engine tests.
//!
//! Stands in for everything the engine talks to — Google OAuth, the Colab
//! control plane (`/tun/m/*`, `v1/*`), a runtime's Jupyter server, the kernel
//! WebSocket and `/colab/tty` — so integration and desktop E2E tests exercise
//! the real client code without a Google account. The servers are added
//! alongside the client features they back (see PLAN.md, phases 1–3).
