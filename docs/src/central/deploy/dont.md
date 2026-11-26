# What not to do

- **Don't put a reverse proxy in front.** reef reads no `X-Forwarded-*` header, so behind a
  proxy every caller looks like the server's own host. That lets anyone add the first user, and
  it puts every caller under the one address the sign-in throttle counts. Serve TLS from reef
  itself.
- **Don't put a plain-HTTP proxy in front either,** for browsers. Over plain HTTP a browser
  speaks HTTP/1.1 and keeps six connections to a host across all its tabs, and each plan page
  holds three change streams open, so a second tab leaves the first's reads waiting (C85). reef
  speaks HTTPS on every port for this; tunnel to its port as it is.
- **Don't expect HSTS.** reef sends none: with reef's own certificate, a browser warns until it
  trusts reef's CA, and HSTS would make that warning one that can't be clicked through.
- **Don't leave sign-in off on a shared host.** With sign-in off, anyone who reaches the
  loopback port can write, have the service clone any repo its account can read, or register
  any directory. A web page whose name a DNS rebinding points at 127.0.0.1 is only redirected
  to https at that name, which reef's certificate doesn't cover, so the browser refuses it
  (C80).
