# Use Rust with the vendored herdr runtime

Use Rust for client and server; vendor herdr’s PTY management, libghostty-vt, persistence and server runtime as crates. Performance precedes customizability, type safety and authoring ease. Start with a coarse runtime crate retaining ratatui data types rather than paying for a clean split before proving the spike.

Accepted by Tommy on 2026-10-10; source: Kizn design record, revision 3. Number allocated in this new repository with no remotes or default branch; recheck against the remote default branch before the first PR.
