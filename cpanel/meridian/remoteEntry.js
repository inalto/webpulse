// WebPulse Analytics - Meridian hub.
//
// Meridian's side navigation is built from hub manifests (_assets/_registry.js);
// every hub is an ES-module Module Federation remote that the shell imports,
// initialises with its share scope (React 19 singleton) and asks for "./module".
// The first exported function whose name ends in "Hub" is rendered at nav.path.
//
// This file is hand-written (no build step): the component embeds the regular
// WebPulse dashboard, served by cPanel as the account through
// webpulse/frame.live.php, in a same-origin iframe sized to its content.

let shareScope = null;

export function init(scope) {
  shareScope = scope && scope.react ? scope : (scope && scope.default) || scope;
  return undefined;
}

async function sharedReact() {
  const versions = shareScope && shareScope.react;
  if (!versions) {
    throw new Error('[webpulse] React is not in the share scope');
  }
  const key = Object.keys(versions)[0];
  const entry = versions[key];
  let mod = typeof entry.lib === 'function' ? entry.lib() : await entry.get();
  if (typeof mod === 'function') {
    mod = mod();
  }
  mod = await mod;
  return mod && mod.default && mod.default.createElement ? mod.default : mod;
}

function frameUrl() {
  const page = window.PAGE || {};
  const base = page.basePath || ((page.securityToken || '') + '/frontend/meridian');
  return base + '/webpulse/frame.live.php';
}

async function buildModule() {
  const React = await sharedReact();
  const h = React.createElement;

  function WebPulseHub() {
    const ref = React.useRef(null);
    const [height, setHeight] = React.useState(900);
    React.useEffect(() => {
      const el = ref.current;
      if (!el) {
        return undefined;
      }
      const fit = () => {
        try {
          const doc = el.contentDocument;
          if (doc && doc.documentElement) {
            const body = doc.body ? doc.body.scrollHeight : 0;
            const next = Math.max(doc.documentElement.scrollHeight, body);
            if (next > 200) {
              setHeight(next + 24);
            }
          }
        } catch (e) {
          // not same-origin (should not happen): keep the fallback height
        }
      };
      el.addEventListener('load', fit);
      const timer = window.setInterval(fit, 500);
      return () => {
        el.removeEventListener('load', fit);
        window.clearInterval(timer);
      };
    }, []);
    return h(
      'div',
      { className: 'webpulse-hub', style: { width: '100%', minHeight: '60vh' } },
      h('iframe', {
        ref,
        src: frameUrl(),
        title: 'WebPulse Analytics',
        loading: 'eager',
        style: { width: '100%', height: height + 'px', border: 0, display: 'block', background: 'transparent' },
      })
    );
  }

  return { WebPulseHub, default: WebPulseHub, __esModule: true };
}

let modulePromise = null;

export function get(name) {
  if (name !== './module' && name !== 'module') {
    return Promise.reject(new Error('[webpulse] unknown exposed module ' + name));
  }
  if (!modulePromise) {
    modulePromise = buildModule();
  }
  return modulePromise.then((m) => () => m);
}
