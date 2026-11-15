# RUX authorization: a generic language feature, not a predefined role hierarchy

Status: **declarative security is implemented for the experimental single-entity CRUD app parser and native runtime; arbitrary application routes remain planned**.

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

## Declarative RUX source syntax (implemented for CRUD apps)

An optional security block can appear alongside a data declaration in a RUX app:

```rux
app SecurityExample {
    data Customer {
        name: Text
        email: Email
    }

    security {
        role physician {
            allow patients.read
            allow patients.update
        }

        route GET "/api/records" {
            require patients.read
        }

        route PUT "/api/records/{id}" {
            require patients.update
        }
    }
}
```

- Security declarations are optional. Existing apps without a security block behave as before.
- Role names are free-form identifiers. Newly declared roles have only the explicitly listed grants.
- The separate CRUD parser currently supports `GET /api/records`, `POST /api/records`, `PUT /api/records/{id}` and `DELETE /api/records/{id}` route policy bindings. Unknown routes are rejected rather than silently treated as protected.
- Unspecified CRUD bindings continue to use the built-in action permission names (`records.read`, `records.create`, `records.update`, `records.delete`).
- Declared roles are synchronized to SQLite transactionally at startup. Grants removed from the source are revoked when the app restarts. CLI grants to source-defined roles are overwritten on the next sync to avoid ambiguous precedence.
- An explicit route requirement cannot run under the legacy anonymous/shared-token mode. It requires `RUX_AUTH_MODE=users` or `RUX_AUTH_MODE=identities`.
- Runtime user sessions, API identities, and tenant memberships evaluate the same explicit route permission. The generated browser UI receives the route binding so visible actions reflect the actual permission.

### First-time setup

The application needs a role in SQLite before a login user can be created, so use the provisioning command once on a trusted administrative machine:

```powershell
cargo run --release -- check examples/secure-crud.rux
cargo run --release -- policy apply examples/secure-crud.rux
cargo run --release -- user create SecurityExample doctor_1 physician
$env:RUX_AUTH_MODE = "users"
cargo run --release -- run examples/secure-crud.rux
```

The `policy apply` command creates the application database if necessary and writes the declared role grants. It does not start a server. Back up existing databases before significant policy changes.

## Compiler/runtime contract

1. The CRUD parser verifies role/permission identifier syntax, duplicates, and supported route bindings. Unknown custom permission names remain valid identifiers; arbitrary handler binding is a future compiler feature.
2. Every route explicitly declared in a security block is bound to its required capability. Undeclared CRUD operations use their existing default capability bindings, with deny-by-default for roles that lack those grants.
3. Frontend visibility is merely UX. Each request is authorized again in the backend and within data queries.
4. Ownership, organization membership and operation grants are checked together without a client-controlled bypass.
5. Resource names must be generic; they should never require a fixed schema such as `Customer`, `Company`, or `User` unless the application requests those modules.
6. Changes to policies are auditable. A future signed/append-only audit stream is separate from operational logs.
7. SDKs and APIs can use the same engine with namespaced capabilities; no Java, Spring, or other framework is required.

## Known gaps

- The main general-purpose RUX parser has not yet been unified with the CRUD security parser.
- Only the native CRUD maps HTTP operations to permission identifiers. Arbitrary RUX routes and reusable security modules still need binding support.
- Custom policy definitions are currently project-wide. Multiple tenants may assign different roles, but roles with the same name share grant definitions in this prototype.
- The historical SQLite role constraints remain in compatibility columns; this is documented migration debt, not a public-language limit.
- No independent security audit has been conducted. Do not deploy the experimental authorization runtime for sensitive production data yet.
