# RUX authorization: a generic language feature, not a predefined role hierarchy

Status: **runtime prototype implemented; declarative RUX syntax proposed**.

## Design principles

RUX is a general-purpose programming language. **No domain-specific roles or companies are mandatory.** Terms such as `reader`, `editor`, `admin`, `doctor`, `teacher`, `tenant`, and `company` are not privileged language keywords.

- Roles are **application-defined names**.
- Permissions are **application-defined capabilities**, with a resource and an action such as `invoices.approve` or `patients.read`.
- Roles start with **zero permissions**. Grants must be explicit.
- The runtime evaluates capabilities, not role names.
- Tenant scopes are **optional**. Apps without multi-tenancy use user-level roles. Tenant-specific grants never imply cross-tenant access.
- Dynamic permission revocation is checked against the database for each authorized request.
- A role may be named `administrator` without having any privilege; the name alone is meaningless to RUX.
- The `reader`, `editor`, and `admin` roles are **legacy compatibility presets**, not a constraint on new applications.

## Runtime prototype (implemented)

```powershell
cargo run --release -- role create teste physician
cargo run --release -- role allow teste physician records.read
cargo run --release -- role allow teste physician patients.read
cargo run --release -- role deny teste physician records.read
cargo run --release -- role list teste
cargo run --release -- user create teste doctor_1 physician
```

Current automatic CRUD permission bindings are `records.read`, `records.create`, `records.update`, `records.delete`, and `metrics.read`. `records.access_all` permits cross-user **non-tenant** record administration when explicitly granted. The runtime **does not** treat custom permission names as routes without a corresponding route policy.

The SQLite migration keeps the older role column and adds `policy_role` for custom roles, without erasing existing users, tokens, memberships, or records. Older executables are not a supported rollback mechanism after custom roles are provisioned.

## Proposed RUX source syntax (NOT YET IMPLEMENTED)

For example, in an optional `security` module:

```text
security {
    role physician {
        allow patients.read
        allow consultations.create
    }

    role receptionist {
        allow appointments.read
        allow appointments.create
    }

    route GET "/patients" {
        require patients.read
    }
}
```

The final syntax will be designed with the main RUX parser and type checker. This example is **not yet accepted by the RUX compiler**.

## Compiler/runtime contract

1. The compiler resolves declared permission identifiers and rejects references to malformed or unknown permission names within an application.
2. Every protected route is bound to a capability. Defaults are deny-by-default, even for roles named `admin`.
3. Frontend visibility is merely UX. Each request is authorized again in the backend and within data queries.
4. Ownership, organization membership and operation grants are checked together without a client-controlled bypass.
5. Resource names must be generic; they should never require a fixed schema such as `Customer`, `Company`, or `User` unless the application requests those modules.
6. Changes to policies are auditable. A future signed/append-only audit stream is separate from operational logs.
7. SDKs and APIs can use the same engine with namespaced capabilities; no Java, Spring, or other framework is required.

## Known gaps

- There is no `security` parser implementation yet.
- Only the native CRUD maps HTTP operations to permission identifiers; arbitrary RUX routes still need binding support.
- Custom policy definitions are currently project-wide. Multiple tenants may assign different roles, but roles with the same name share grant definitions in this prototype.
- The historical SQLite role constraints remain in compatibility columns; this is documented migration debt, not a public-language limit.
- No independent security audit has been conducted. Do not deploy the experimental authorization runtime for sensitive production data yet.
