# What not to do

- **Don't put a reverse proxy in front.** reef reads no `X-Forwarded-*` header, so behind a
  proxy every caller looks like the server's own host. That lets anyone add the first user, and
  it puts every caller under the one address the sign-in throttle counts. Serve TLS from reef
  itself.
- **Don't serve browsers from another machine over plain HTTP,** such as through an SSH tunnel
  to the loopback port. Over plain HTTP a browser speaks HTTP/1.1 and keeps at most six
  connections to a host across all its tabs, and each plan page holds three change streams
  open, so with a second tab open the first's reads wait and its sessions panel shows nothing
  (C85). Reach the TLS port instead, tunnelled or not: over TLS the browser speaks HTTP/2, and
  every tab's streams and reads share one connection.
- **Don't expect HSTS.** reef sends none, since the plain loopback port shares the browser's
  idea of the host.
- **Don't leave sign-in off on a shared host.** With sign-in off, anyone who reaches the
  loopback port can write, have the service clone any repo its account can read, or register
  any directory. A web page whose name a DNS rebinding points at 127.0.0.1 counts as the plain
  port's own origin too (C80).
