# RLS-POLICIES.md

Row-level security rules and policy patterns for this project. Any AI coding
agent working on this repository follows this file.

Applies to every Postgres table in a schema reachable outside a trusted
server context, and as defence in depth for server-only access. Storage
buckets follow the same ownership model.

## Hard rules (never break)

1. Row Level Security is ENABLED on every table in every exposed schema.
   RLS enabled with no policies blocks everything; RLS disabled allows
   everything. Enabled-with-policies is the only correct state.
2. Separate policies per command. SELECT, INSERT, UPDATE and DELETE each
   get their own policy. INSERT and UPDATE policies carry WITH CHECK so a
   user cannot create rows owned by someone else or change `user_id` /
   `owner_id` on an existing row.
3. Never base a policy on client-editable metadata (for example Supabase
   `user_metadata`). Users can edit it and promote themselves. Use server
   columns or server-only metadata (`app_metadata`, custom claims, a roles
   table).
4. Views bypass RLS unless created with `security_invoker = true`. Every
   view over a protected table sets it.
5. SECURITY DEFINER functions bypass RLS, so they must check permissions
   themselves, and they pin their `search_path`.
6. The service-role / admin key and the database connection string are used
   only in server code. The app connects as a least-privilege role, never a
   superuser.
7. Deny by default: a new table gets RLS enabled and explicit policies
   before it holds real data. "We'll add policies later" is a leak.
8. Table owners bypass RLS unless `FORCE ROW LEVEL SECURITY` is set. If
   migrations or the app connect as the table owner, use FORCE.

## User context

Policies need the identity of the current requester. This project defines
one helper and every policy uses it:

```sql
create schema if not exists app;

create or replace function app.current_user_id()
returns uuid language sql stable as $$
  select nullif(current_setting('app.current_user_id', true), '')::uuid
$$;
```

The server verifies the session or token first, then sets the context per
request or transaction:

```sql
set local app.current_user_id = '00000000-0000-0000-0000-000000000000';
```

In Supabase-style setups `auth.uid()` fills this role; the pattern is
identical. The identity always comes from a verified session, never from
client input.

## Pattern library

### 1. User-owned rows

The default for most tables: orders, notes, items, files, settings.

```sql
alter table public.items enable row level security;

create policy "items_select_own" on public.items
  for select using (user_id = app.current_user_id());

create policy "items_insert_own" on public.items
  for insert with check (user_id = app.current_user_id());

create policy "items_update_own" on public.items
  for update using (user_id = app.current_user_id())
  with check (user_id = app.current_user_id());

create policy "items_delete_own" on public.items
  for delete using (user_id = app.current_user_id());
```

The `user_id` column is set by the server from the verified session, never
accepted from client input.

### 2. Admin-only tables

Role lives in a trusted source (roles table, server-only metadata claim):

```sql
create or replace function app.is_admin()
returns boolean language sql stable as $$
  select exists (
    select 1 from public.user_roles r
    where r.user_id = app.current_user_id() and r.role = 'admin'
  )
$$;

create policy "audit_log_admin_only" on public.audit_log
  for select using (app.is_admin());
-- no insert/update/delete policies at all: nothing outside
-- the service role can write to it
```

### 3. Public read, owner-managed

Published content readable by everyone, writable only by the owner:

```sql
create policy "posts_select_public" on public.posts
  for select using (is_published or user_id = app.current_user_id());

create policy "posts_write_own" on public.posts
  for insert with check (user_id = app.current_user_id());

create policy "posts_update_own" on public.posts
  for update using (user_id = app.current_user_id())
  with check (user_id = app.current_user_id());

create policy "posts_delete_own" on public.posts
  for delete using (user_id = app.current_user_id());
```

### 4. Profiles: read all, write own

Readable by every authenticated user, but role and privilege columns are
not writable through the public path:

```sql
create policy "profiles_select_all" on public.profiles
  for select using (app.current_user_id() is not null);

create policy "profiles_update_own" on public.profiles
  for update using (id = app.current_user_id())
  with check (id = app.current_user_id() and role = app.user_role(id));
```

If keeping `role` immutable through RLS is not practical, move privileged
columns to a separate admin-managed table and leave the public table
clean.

### 5. Shared group rows (family, team, household)

Access through a membership junction table. Index the membership columns;
the EXISTS runs per row:

```sql
create policy "docs_select_member" on public.docs
  for select using (
    exists (
      select 1 from public.group_members m
      where m.group_id = docs.group_id
        and m.user_id = app.current_user_id()
    )
  );

create policy "docs_insert_member" on public.docs
  for insert with check (
    exists (
      select 1 from public.group_members m
      where m.group_id = docs.group_id
        and m.user_id = app.current_user_id()
        and m.role in ('owner', 'member')
    )
  );
```

Decide per table whether every member can delete, or only the owner and an
admin; encode that choice in the DELETE policy. Update joins on group
membership (invite, kick, role change) are admin-only or owner-only.

### 6. Append-only tables (logs, events)

No UPDATE or DELETE policies at all. The service role writes, users read
their own rows, admins read everything:

```sql
create policy "events_select_own" on public.events
  for select using (user_id = app.current_user_id());
```

### 7. Reference / lookup data

Static data readable by everyone, writable by admins only:

```sql
create policy "plans_select_all" on public.plans
  for select using (true);
-- writes happen only through the service role or admin functions
```

## Decision table

| Table type | SELECT | INSERT | UPDATE | DELETE | Notes |
|---|---|---|---|---|---|
| User-owned (default) | own rows | own, WITH CHECK | own, WITH CHECK | own | `user_id` set server-side |
| Profiles | all authenticated | own | own, role fields locked | own | split out privileged columns |
| Admin-only / audit | admin | none (service role) | none | none | no public write path |
| Public content | public (or condition) | owner | owner | owner | WITH CHECK on every write |
| Shared group | member (EXISTS) | member, WITH CHECK | per role decision | owner or admin per decision | index membership FKs |
| Append-only | own or admin | service role | none | none | no UPDATE policy exists |
| Reference data | public | none | admin only | none | service role for seeds |

New table: pick the row above, copy the pattern, adjust the predicate. If
none fits, stop and design the access model before writing the table.

## Classic bypasses (checked on every review)

- View over a protected table without `security_invoker = true`.
- SECURITY DEFINER function without its own permission check.
- Policy based on client-editable metadata.
- Missing WITH CHECK on INSERT or UPDATE.
- A second connection role (service role) used from client-reachable code.
- Public storage bucket with listing enabled.
- Table owner bypassing RLS (no FORCE) while the app connects as owner.
- Function default `search_path` letting a caller shadow objects; pin it:
  `set search_path = public, app`.

## Server-only access still needs ownership checks

When the app reaches Postgres only through its own server, RLS is the
second wall, not the first. Every query that fetches, updates or deletes
by ID must still scope to the current user or check permission explicitly.
Changing an ID in the request (`/orders/18` to `/orders/22`) must return
403 or 404, never another user's row. This remains the most common
real-world defect (IDOR).

Also for the connection itself: least-privilege role, TLS required, not
reachable from the public internet (IP allowlist), automated backups on.
API responses never include sensitive columns: password hashes, tokens,
internal flags, other users' emails. Select only the fields the UI needs.

## Storage

- Buckets are private by default; per-user and per-group folders with
  policies so users access only their own files.
- Public buckets are never listable.
- Uploads follow `SECURITY.md` (type, size, naming, serving).

## Self-test

- Create two accounts, A and B. As A, copy the request that loads A's data,
  swap in B's ID, send: expect 403 or 404. Repeat for update and delete.
- Signed out, using only the public/anon key: nothing private comes back.
  Any table with data visible this way has a missing or broken policy.
- A user cannot change `user_id`/`owner_id` on their own row through an
  UPDATE.
- List policies and compare against the decision table:
  every table in exposed schemas shows RLS enabled and a policy per
  command. Run the provider's security advisor if one exists and clear
  every warning.
- A view over a protected table leaks nothing to a signed-out user.
- Service-role key appears only in server code.

## Done when

- RLS enabled on every table in exposed schemas, with per-command policies
  from the pattern library.
- WITH CHECK present on every INSERT and UPDATE policy.
- Roles and ownership come from server-trusted sources.
- Views use `security_invoker`; SECURITY DEFINER functions check
  permissions themselves.
- Cross-user access tested and blocked (two-account test).
- Service-role key server-only; connection least-privilege, TLS, not
  internet-open; backups on.

Basis: Pre-Launch Security Toolkit, check 03; Postgres RLS model.
