# MySQL user store

Optional, enabled by `M7S_MYSQL_URL`:

```text
M7S_MYSQL_URL=mysql://root@127.0.0.1:3307/m7s
```

Schema:

```sql
CREATE DATABASE m7s;
CREATE TABLE m7s.users (
  username VARCHAR(64) PRIMARY KEY,
  hash VARCHAR(128) NOT NULL
);
```

API:

- `/api/login` checks MySQL first when configured;
- `/api/users/add` writes MySQL when configured.

Verified with MariaDB 11.4.5 on port 3307:

```text
mysql_user_roundtrip ... ok
```

Test command:

```powershell
$env:M7S_MYSQL_URL='mysql://root@127.0.0.1:3307/m7s'
cargo test -p m7s-rs --test mysql_users
```
