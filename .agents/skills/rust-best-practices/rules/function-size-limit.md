---
title: Keep Functions Under ~50 Lines and Single-Purpose
impact: MEDIUM
tags: code-quality, function-size, single-responsibility, testing
---

## Keep Functions Under ~50 Lines and Single-Purpose

Long functions hide bugs. Once a function exceeds ~50 lines or does more than one thing, individual steps stop being testable in isolation and reviewers stop reading carefully. The cure is almost always to extract pure helpers — small functions that take inputs, return outputs, and have no side effects — leaving the original function as a thin orchestrator.

**Incorrect (one 60-line function doing parse, validate, normalize, persist):**

```rust
pub fn import_user(raw: &str, db: &Db) -> Result<UserId, ImportError> {
    let mut parts = raw.split(',');
    let name = parts.next().ok_or(ImportError::MissingName)?.trim().to_string();
    let email_raw = parts.next().ok_or(ImportError::MissingEmail)?.trim();
    if !email_raw.contains('@') || email_raw.starts_with('@') {
        return Err(ImportError::InvalidEmail(email_raw.to_string()));
    }
    let email = email_raw.to_lowercase();
    let plan_raw = parts.next().unwrap_or("free").trim();
    let plan = match plan_raw {
        "free" => Plan::Free,
        "pro" => Plan::Pro,
        "ent" | "enterprise" => Plan::Enterprise,
        other => return Err(ImportError::UnknownPlan(other.to_string())),
    };
    let normalized_name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let row = NewUserRow { name: normalized_name, email, plan };
    let id = db.insert_user(row).map_err(ImportError::Db)?;
    Ok(id)
}
```

**Correct (orchestrator + small pure helpers, each independently testable):**

```rust
pub fn import_user(raw: &str, db: &Db) -> Result<UserId, ImportError> {
    let parsed = parse_user_csv(raw)?;
    let row = normalize_user(parsed)?;
    db.insert_user(row).map_err(ImportError::Db)
}

fn parse_user_csv(raw: &str) -> Result<RawUser, ImportError> { /* ~10 lines */ }

fn normalize_user(raw: RawUser) -> Result<NewUserRow, ImportError> { /* ~10 lines */ }
```

Short, single-purpose functions are easier to name, easier to unit-test, and dramatically easier for a reviewer to trust.
