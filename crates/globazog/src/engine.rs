// Copyright (c) 2026 Mike Grier

//! The Model B async-completion scheduler (D-3): permit-bounded directory scans
//! (D-7, D-8), relative-open + parent-handle refcount (D-9, D-10), depth-first
//! enumerate-then-recurse (D-48), the unifying continuation suspension (D-59),
//! cancellation accounting (D-50), and cycle detection (D-51).
