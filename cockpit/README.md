# WebPulse Cockpit module

The files Cockpit loads from `/usr/share/cockpit/webpulse/`, which gives the
**Web analytics** entry of Cockpit's menu (the WebPulse dashboard).

| File            | Origin                                                                 |
|-----------------|------------------------------------------------------------------------|
| `manifest.json` | this directory; `install.sh` writes the release version into `version` |
| `index.html`    | generated at install time by `webpulse page --cockpit --bin <binary>`  |

`index.html` is the dashboard every other WebPulse front end serves (the same
`ui/*.js` and `ui/app.css`, inlined) plus Cockpit's `../base1/cockpit.js` and
the adapter `ui/cockpit.js`. The adapter does not use HTTP: each API call runs

```
/usr/local/webpulse/bin/webpulse cgi --cockpit --api
```

through `cockpit.spawn(…, {superuser: "try"})`, with the CGI environment
(`REQUEST_METHOD`, `QUERY_STRING`, `CONTENT_LENGTH`, `HTTP_X_WEBPULSE_REQUEST`)
and the request body on stdin, and parses the CGI reply. Cockpit does the
login, the transport and the privilege switch:

* **Administrative access off**: the process runs as the logged-in Linux
  user, who sees the sites it owns (`owner` in `webpulse sites`; on cPanel
  servers the cPanel account of the same name). The statistics files are
  `0640 root:<owner>`, so the operating system enforces it.
* **Administrative access on**: the process runs as root, which is the
  administrator view (every site, the server view, settings and jobs).

`manifest.json` sets the module's Content-Security-Policy to allow the
inline scripts and styles of the page (Cockpit adds its own defaults for the
other directives). Install, update and removal:

```
install.sh --standalone            # standalone servers: implies --cockpit when Cockpit is installed
install.sh --cockpit               # add the module (also on cPanel servers running Cockpit)
install.sh --update                # regenerates index.html when the module is installed
install.sh --uninstall             # removes /usr/share/cockpit/webpulse
```

Cockpit picks modules up without a restart (reload the browser tab). See the
wiki page `Cockpit-Module` and `docs/STANDALONE-COCKPIT.md`.
