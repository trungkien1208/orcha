// The settings page (orcha_settings_web.html), served as /app.js so the
// page runs no inline script.
import { html, render, useState, useEffect, useRef, useCallback } from "/vendor/htm-preact.mjs";

// the server writes the API token into the page; a reload gets in with its cookie
const token = document.querySelector('meta[name="orcha-token"]')?.content || "";
// keep the token out of the address bar, history and screenshots
history.replaceState(null, "", "/" + location.hash);

async function call(path, body) {
  const response = await fetch(path, {
    method: body ? "POST" : "GET",
    headers: { "X-Orcha-Token": token, "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || response.statusText);
  return data;
}

function usePoll(fn, every) {
  useEffect(() => {
    let alive = true;
    const run = () => alive && fn();
    run();
    const id = setInterval(run, every);
    return () => { alive = false; clearInterval(id); };
  }, []);
}

// a link only to a web address: a plugin's url must not become javascript: or file:
const webUrl = (url) => {
  try { const u = new URL(url); return u.protocol === "http:" || u.protocol === "https:" ? u.href : null; }
  catch (_) { return null; }
};
const matches = (q, ...texts) => !q || texts.some((t) => (t || "").toLowerCase().includes(q));
const changed = (f) => "default" in f && JSON.stringify(f.default) !== JSON.stringify(f.value);

// ---- the page wears the orcha theme in use ----
const themeStyle = document.head.appendChild(document.createElement("style"));
function paintTheme(c, accent) {
  if (!c) return;
  const mix = (a, b, p) => `color-mix(in srgb, ${a} ${p}%, ${b})`;
  const a = accent || c.accent;
  themeStyle.textContent = `:root:not(#_):not(#__), [data-theme]:not(#_):not(#__) {
    --accent: ${a}; --text: ${c.text}; --muted: ${c.muted}; --raised: ${c.raised};
    --pico-background-color: ${c.bg}; --pico-card-background-color: ${c.card}; --pico-card-sectioning-background-color: ${c.raised};
    --pico-muted-border-color: ${mix(c.border, c.card, 40)}; --pico-color: ${c.text}; --pico-muted-color: ${c.muted};
    --pico-h1-color: ${c.text}; --pico-h2-color: ${c.text}; --pico-h3-color: ${c.text};
    --pico-form-element-background-color: ${c.raised}; --pico-form-element-border-color: ${mix(c.border, c.card, 60)};
    --pico-form-element-color: ${c.text}; --pico-form-element-placeholder-color: ${c.muted};
    --pico-form-element-active-background-color: ${c.raised}; --pico-form-element-active-border-color: ${a};
    --pico-switch-background-color: ${mix(c.border, c.card, 80)}; --pico-switch-checked-background-color: ${a};
    --pico-range-border-color: ${mix(c.border, c.card, 70)}; --pico-range-active-border-color: ${mix(c.border, c.card, 90)};
    --pico-range-thumb-color: ${a}; --pico-range-thumb-active-color: ${a};
    --pico-code-background-color: ${c.raised}; --pico-code-color: ${c.text};
    --yellow: ${c.yellow}; --green: ${c.green}; --red: ${c.red};
  }`;
}

// ---- controls ----
function Seg({ f, save }) {
  return html`<div class="seg" role="group">${f.options.map((o) => html`
    <button class=${o.value === f.value ? "on" : ""} onClick=${() => o.value !== f.value && save(f.key, o.value)}>${o.label}</button>`)}</div>`;
}

function Range({ f, save }) {
  const [v, setV] = useState(f.value);
  useEffect(() => setV(f.value), [f.value]);
  return html`<div class="range">
    <input type="range" min=${f.min} max=${f.max} value=${v} aria-label=${f.label}
      onInput=${(e) => setV(Number(e.target.value))} onChange=${(e) => save(f.key, Number(e.target.value))} />
    <output>${v}${f.unit || ""}</output></div>`;
}

function Text({ f, save }) {
  const [v, setV] = useState(f.value);
  // Escape blurs to leave the field; the blur must not save what it threw away
  const cancelled = useRef(false);
  useEffect(() => setV(f.value), [f.value]);
  const blur = (e) => {
    if (cancelled.current) { cancelled.current = false; return; }
    // the field itself, not `v`: a blur right after typing can beat the re-render
    if (e.target.value !== f.value) save(f.key, e.target.value);
  };
  return html`<input class="grow" type="text" value=${v} spellcheck=${false} aria-label=${f.label}
    onInput=${(e) => setV(e.target.value)} onBlur=${blur}
    onKeyDown=${(e) => {
      if (e.key === "Enter") e.target.blur();
      if (e.key === "Escape") { cancelled.current = true; e.target.value = f.value; setV(f.value); e.target.blur(); }
    }} />`;
}

const Strip = ({ colors }) => html`<div class="strip">${(colors || []).map((c) => html`<i style=${{ background: c || "transparent" }} />`)}</div>`;

/** Themes and palettes: a dropdown, with the chosen one's colors beside it. */
function Picker({ f, save }) {
  const chosen = f.options.find((o) => o.value === f.value);
  const groups = f.kind === "themes" ? [["dark", "Dark"], ["light", "Light"]] : [[null]];
  return html`<${Strip} colors=${chosen?.swatch} />
    <select class="grow" aria-label=${f.label} onChange=${(e) => save(f.key, e.target.value)}>
      ${groups.map(([g, name]) => {
        const opts = f.options.filter((o) => !g || o.help === g).map((o) => html`<option value=${o.value} selected=${o.value === f.value}>${o.label}</option>`);
        return g ? html`<optgroup label=${name}>${opts}</optgroup>` : opts;
      })}
    </select>`;
}

function Control({ f, save }) {
  switch (f.kind) {
    case "toggle":
      return html`<input type="checkbox" role="switch" checked=${!!f.value} aria-label=${f.label} onChange=${(e) => save(f.key, e.target.checked)} />`;
    case "range": return html`<${Range} f=${f} save=${save} />`;
    case "text": return html`<${Text} f=${f} save=${save} />`;
    case "themes": case "palettes": return html`<${Picker} f=${f} save=${save} />`;
    case "swatches":
      return html`<div class="swatches">${f.options.map((o) => html`
        <button class=${(o.value === f.value ? "on " : "") + (o.color ? "" : "auto")} title=${o.color ? o.label : "the theme's own"} aria-label=${o.label}
          style=${o.color ? { background: o.color } : {}} onClick=${() => o.value !== f.value && save(f.key, o.value)} />`)}</div>`;
    case "select":
      if (f.options.length <= 4) return html`<${Seg} f=${f} save=${save} />`;
      return html`<select class="grow" aria-label=${f.label} onChange=${(e) => save(f.key, e.target.value)}>
        ${f.options.map((o) => html`<option value=${o.value} selected=${o.value === f.value}>${o.label}</option>`)}</select>`;
    default: return html`<span class="muted">${String(f.value)}</span>`;
  }
}

function Row({ f, save }) {
  const c = changed(f);
  return html`<div class="row">
    <div class="label">
      <b>${f.label}${c && html`<span class="dot" title="changed" />`}</b>
      ${f.help && html`<small>${f.help}</small>`}
    </div>
    <div class="ctl">
      <${Control} f=${f} save=${save} />
      ${c && html`<button class="reset" title="back to the default" aria-label=${"reset " + f.label} onClick=${() => save(f.key, f.default)}>↺</button>`}
    </div>
    ${f.warn && html`<div class="warn">${f.warn}</div>`}
  </div>`;
}

/** Rows under their sub-headings. */
function Grouped({ fields, save, skip = [] }) {
  const out = [];
  let group = null;
  for (const f of fields) {
    if (skip.includes(f.group)) continue;
    if (f.group !== group) { group = f.group; out.push(html`<div class="sub">${group}</div>`); }
    out.push(html`<${Row} key=${f.key} f=${f} save=${save} />`);
  }
  return out;
}

// ---- layout: a miniature orcha window, drawn in the chosen theme ----
function Sketch({ get }) {
  const theme = get("theme");
  const [bg = "#181e16", panel = "#262e22", text = "#e8f0d6", themeAccent = "#ff8fab", green = "#96d660", yellow = "#f2c53d"] =
    theme?.options.find((o) => o.value === theme.value)?.swatch || [];
  const accentField = get("accent");
  const accent = accentField?.options.find((o) => o.value === accentField.value)?.color || themeAccent;
  const v = (k) => get(k)?.value;
  const panelOn = !!v("right_panel"), fuel = v("fuel"), proc = v("process");
  const width = { narrow: "22%", normal: "27%", wide: "33%" }[v("sidebar_width")] || "27%";
  const dim = (a) => `color-mix(in srgb, ${text} ${a}%, transparent)`;
  const paneBg = v("pane_paper") ? panel : bg;
  const train = html`<div class="train" style=${{ background: `repeating-linear-gradient(90deg, ${yellow} 0 7px, transparent 7px 10px)` }} />`;
  const gauge = (pct, label) => html`<div class="gauge" style=${{ color: dim(70) }}>${label}<div style=${{ background: dim(15) }}><b style=${{ width: pct + "%", background: green }} /></div></div>`;
  const projects = [["api", true], ["web", false], ["docs", false]].map(([name, on]) => html`
    <div class="proj" style=${{ background: on ? `color-mix(in srgb, ${accent} 22%, transparent)` : "transparent", color: on ? text : dim(60) }}>
      <b style=${{ background: on ? accent : dim(35) }} />${name}</div>`);
  const side = html`<div class="side" style=${{ width, background: bg, color: dim(55) }}>PROJECTS ${projects}
    ${(proc === "sidebar" || (proc === "panel" && !panelOn)) && train}</div>`;
  const right = panelOn && html`<div class="panel" style=${{ background: bg, color: dim(55) }}>FUEL
    ${fuel === "panel" && [gauge(72, "work"), gauge(38, "home")]}
    ${proc === "panel" && train}</div>`;
  const pane = (lines) => html`<div class="pane" style=${{ background: paneBg, boxShadow: `inset 0 0 0 1px ${dim(10)}` }}>
    ${lines.map((w, i) => html`<div class="bar" style=${{ width: w + "%", background: i === 0 ? accent : dim(i === lines.length - 1 ? 30 : 18) }} />`)}</div>`;
  const panes = html`<div class="panes" style=${{ background: bg }}>${pane([46, 80, 64, 72, 38])}${pane([40, 70, 86, 52])}</div>`;
  return html`<div class="preview">
    <div class="win">
      <div class="titlebar" style=${{ background: bg }}>
        <i style=${{ background: "#ff5f57" }} /><i style=${{ background: "#febc2e" }} /><i style=${{ background: "#28c840" }} />
        <span style=${{ background: accent }}>⌕ commands</span>
      </div>
      <div class="body">${v("sidebar_side") === "right" ? [right, panes, side] : [side, panes, right]}</div>
      <div class="status" style=${{ background: bg }}>${fuel === "status_bar" && gauge(72, "work")}</div>
    </div>
    <small>This is your orcha window in ${theme?.value}. It changes as you do.</small>
  </div>`;
}

// ---- agents: the accounts, and what happens as one runs low ----
function Accounts({ data, setData, toast }) {
  const [name, setName] = useState("");
  const [armed, setArmed] = useState("");
  const [busy, setBusy] = useState("");
  const act = async (action, body = {}) => {
    setBusy(action + (body.id || ""));
    try { const r = await call("/api/accounts/" + action, body); setData(r.state); toast(r.note); if (action === "add") setName(""); }
    catch (err) { toast(err.message, true); }
    setBusy("");
  };
  const color = (left) => left < 15 ? "var(--red)" : left < 40 ? "var(--yellow)" : "var(--green)";
  if (!data) return html`<p class="muted">Reading your accounts…</p>`;
  return html`<div>
    ${data.accounts.map((a) => {
      const out = a.signed_in === false;
      return html`<div class="acct">
        <div class="who">
          <b>${a.label}</b>${a.current && html`<span class="tag">in use</span>`}${a.default && !a.current && html`<span class="tag dim">default</span>`}
          <small>${out ? "signed out" : a.error ? a.error : a.email || a.kind}</small>
        </div>
        <div class="meters">${a.windows.length ? a.windows.map((w) => html`
          <div class="meter" title=${w.reset ? "refills " + w.reset : ""}><span>${w.label.toLowerCase()}</span>
            <div><b style=${{ width: w.left + "%", background: color(w.left) }} /></div><span>${w.left}%</span></div>`)
          : html`<small>${out ? "sign in to see what is left" : "no plan limits for this one"}</small>`}</div>
        <div class="acts">
          ${out ? html`<button class="btn" disabled=${!!busy} onClick=${() => act("login", { id: a.id })}>Sign in</button>`
                : !a.default && html`<button class="btn ghost" disabled=${!!busy} onClick=${() => act("default", { id: a.id })} title="new agents start on this account">Use for new</button>`}
          <button class="btn ghost" disabled=${!!busy} onClick=${() => {
            if (armed === a.id) { setArmed(""); act("remove", { id: a.id }); } else { setArmed(a.id); setTimeout(() => setArmed(""), 4000); }
          }}>${armed === a.id ? "Remove?" : "Remove"}</button>
        </div>
      </div>`;
    })}
    <div class="addrow">
      <input type="text" placeholder="new Claude account, e.g. work" value=${name} onInput=${(e) => setName(e.target.value)}
        onKeyDown=${(e) => e.key === "Enter" && name.trim() && act("add", { name })} aria-label="new account name" />
      <button class="btn ghost" disabled=${!name.trim() || !!busy} onClick=${() => act("add", { name })}>Add and sign in</button>
      <span style=${{ flex: 1 }} />
      <button class="btn ghost" disabled=${!!busy} onClick=${() => act("refresh")}>${busy === "refresh" ? "Checking…" : "Check fuel now"}</button>
    </div>
  </div>`;
}

function Stages({ get, save }) {
  const v = (k) => Number(get(k)?.value || 0);
  const stage = (key, when, title, help, canOff, fallback) => {
    const f = get(key);
    const on = v(key) > 0;
    const set = (n) => save(key, Math.max(1, Math.min(f?.max || 60, n)));
    return html`<div class=${"stage" + (on ? "" : " off")}>
      <div class="stage-head"><span class="when">${when}</span>
        ${canOff && html`<input type="checkbox" role="switch" checked=${on} aria-label=${title} onChange=${(e) => save(key, e.target.checked ? fallback() : 0)} />`}</div>
      <div class="big">${on ? html`<button aria-label="less" onClick=${() => set(v(key) - 5)}>−</button><span>${v(key)}%</span><button aria-label="more" onClick=${() => set(v(key) + 5)}>+</button>` : html`<span>off</span>`}</div>
      <b>${title}</b><small>${help}</small>
      ${f?.warn && html`<div class="warn">${f.warn}</div>`}
    </div>`;
  };
  return html`<div class="stages">
    ${stage("checklist_percent", "first", "Offer a checklist", "the agent writes down what is done and what is next", true, () => Math.max(25, v("low_fuel_percent") + 10))}
    <span class="arrow">→</span>
    ${stage("low_fuel_percent", "then", "Ask where to continue", "orcha suggests another account or agent", false)}
    <span class="arrow">→</span>
    ${stage("auto_handoff_percent", "last", "Move on by itself", "the chat goes to the account with the most left", true, () => Math.max(1, Math.floor(v("low_fuel_percent") / 2)))}
  </div>
  <small>Each step runs once per account, while the agent is waiting for you.</small>`;
}

// ---- sections ----
function SettingsSection({ section, state, accounts, setAccounts, toast, save, q }) {
  const all = state.sections.flatMap((s) => s.fields);
  const get = (key) => all.find((f) => f.key === key);
  const hit = matches(q, section.title);
  const fields = section.fields.filter((f) => hit || matches(q, f.label, f.help, f.key, f.group));
  if (!fields.length) return null;
  const head = html`<div class="head"><h2>${section.title}</h2><p>${section.intro}</p></div>`;
  let body;
  if (section.id === "layout" && !q) {
    const panels = fields.filter((f) => f.group === "Panels");
    const extras = fields.filter((f) => f.group === "Extras");
    body = html`<div class="look"><${Sketch} get=${get} /><div class="rows">${panels.map((f) => html`<${Row} key=${f.key} f=${f} save=${save} />`)}</div></div>
      <div class="sub">Extras</div>
      <div class="twocol">${extras.filter((f) => f.kind === "toggle").map((f) => html`<${Row} key=${f.key} f=${f} save=${save} />`)}</div>
      ${extras.filter((f) => f.kind !== "toggle").map((f) => html`<${Row} key=${f.key} f=${f} save=${save} />`)}`;
  } else if (section.id === "agents" && !q) {
    const low = fields.filter((f) => f.group === "When an account runs low");
    body = html`<div class="sub">Accounts</div>
      <${Accounts} data=${accounts} setData=${setAccounts} toast=${toast} />
      <${Grouped} fields=${fields} save=${save} skip=${["When an account runs low"]} />
      <div class="sub">When an account runs low</div>
      <${Stages} get=${get} save=${save} />
      ${low.filter((f) => f.key === "switch_mode").map((f) => html`<${Row} key=${f.key} f=${f} save=${save} />`)}`;
  } else if (section.id === "advanced" && !q) {
    body = html`<details class="more"><summary>Show ${fields.length} settings</summary><${Grouped} fields=${fields} save=${save} /></details>`;
  } else {
    body = html`<${Grouped} fields=${fields} save=${save} />`;
  }
  return html`<article id=${section.id}>${head}${body}</article>`;
}

// ---- projects: rules and skills ----
const Diff = ({ text }) => html`<pre class="diff">${text.split("\n").map((l) => html`<span class=${l.startsWith("+ ") ? "add" : l.startsWith("- ") ? "del" : ""}>${l + "\n"}</span>`)}</pre>`;

function RuleItem({ rule, on, toggle }) {
  const [open, setOpen] = useState(false);
  return html`<div class="item">
    <input type="checkbox" role="switch" checked=${on} aria-label=${rule.title} onChange=${(e) => toggle(rule.id, e.target.checked)} />
    <div><b>${rule.title}</b><small>${rule.summary}</small></div>
    <button class="text" onClick=${() => setOpen(!open)}>${open ? "hide" : "read"}</button>
    ${open && html`<pre>${rule.body}</pre>`}
  </div>`;
}

function Projects({ toast, q }) {
  const [data, setData] = useState(null);
  const [sel, setSel] = useState("__new");
  const [info, setInfo] = useState(null);
  const [chosen, setChosen] = useState([]);
  const [diff, setDiff] = useState(null);
  const [busy, setBusy] = useState("");

  useEffect(() => {
    call("/api/projects").then((d) => {
      setData(d);
      // `#projects:<folder>` (from orcha's new-project question) opens that one
      const asked = decodeURIComponent((location.hash.match(/^#projects:(.+)$/) || [])[1] || "");
      if (asked && d.projects.some((p) => p.path === asked)) setSel(asked);
    }).catch((err) => toast(err.message, true));
  }, []);
  const load = useCallback(async (path) => {
    setInfo(null); setDiff(null);
    if (path === "__new") return;
    try { const i = await call("/api/projects/show", { path }); setInfo(i); setChosen(i.applied); }
    catch (err) { toast(err.message, true); }
  }, []);
  useEffect(() => { load(sel); }, [sel]);

  if (q && !matches(q, "projects rules skills claude.md agents.md")) return null;
  const head = html`<div class="head"><h2>Projects</h2>
    <p>Rules are short instructions orcha writes into a project's CLAUDE.md, so every agent there follows them. Skills are add-ons an agent can use.</p></div>`;
  if (!data) return html`<article id="projects">${head}<p class="muted">Looking for your projects…</p></article>`;

  const order = (ids) => data.rules.map((r) => r.id).filter((id) => ids.includes(id));
  const isNew = sel === "__new";
  const file = info?.file.split("/").pop();
  const changes = info ? data.rules.filter((r) => chosen.includes(r.id) !== info.applied.includes(r.id)).length : 0;

  const toggleDefault = async (id, on) => {
    const next = order(on ? [...data.defaults, id] : data.defaults.filter((x) => x !== id));
    try { const r = await call("/api/projects/defaults", { rules: next }); setData({ ...data, defaults: next }); toast(r.note); }
    catch (err) { toast(err.message, true); }
  };
  const toggleRule = (id, on) => { setChosen(order(on ? [...chosen, id] : chosen.filter((x) => x !== id))); setDiff(null); };
  const preview = async () => { try { setDiff(await call("/api/projects/preview", { path: sel, rules: chosen })); } catch (err) { toast(err.message, true); } };
  const write = async () => {
    try { const r = await call("/api/projects/apply", { path: sel, rules: chosen }); toast(r.note); load(sel); }
    catch (err) { toast(err.message, true); }
  };
  const install = async (skill, global) => {
    setBusy(skill.name + global);
    try { const r = await call("/api/projects/install", { path: sel, skill: skill.name, global }); toast(r.note); load(sel); }
    catch (err) { toast(err.message, true); }
    setBusy("");
  };

  return html`<article id="projects">${head}
    <div class="pick">
      <select aria-label="project" onChange=${(e) => setSel(e.target.value)}>
        <option value="__new" selected=${isNew}>Every new project (defaults)</option>
        <optgroup label="Your projects">
          ${data.projects.map((p) => html`<option value=${p.path} selected=${sel === p.path}>${p.name}</option>`)}
        </optgroup>
      </select>
      <small>${isNew ? "orcha offers these once when you add a project" : info ? info.file.replace(/^.*\/(?=[^/]+\/[^/]+$)/, "…/") + (info.file_exists ? "" : " (new file)") : "reading…"}</small>
    </div>

    <div class="sub">Rules</div>
    ${isNew
      ? data.rules.map((r) => html`<${RuleItem} key=${r.id} rule=${r} on=${data.defaults.includes(r.id)} toggle=${toggleDefault} />`)
      : info && html`
        ${data.rules.map((r) => html`<${RuleItem} key=${r.id} rule=${r} on=${chosen.includes(r.id)} toggle=${toggleRule} />`)}
        <div class="apply">
          <span class="grow">${changes ? `${changes} ${changes === 1 ? "change" : "changes"}, not written yet` : `${file} is up to date`}</span>
          <button class="btn ghost" onClick=${() => { setChosen(order(data.defaults)); setDiff(null); }}>Use defaults</button>
          <button class="btn" disabled=${!changes} onClick=${diff ? write : preview}>${diff ? "Write to " + file : "Preview"}</button>
        </div>
        ${diff && html`<${Diff} text=${diff.diff} />`}`}

    ${!isNew && info && html`
      <div class="sub">Skills</div>
      <small>Installed with the skills CLI, which needs Node.js. "Here" puts it in this project, "For me" in every project.</small>
      ${data.skills.map((s) => {
        const where = info.skills[s.name] || {};
        const status = where.project ? "in this project" : where.global ? "for you" : "";
        return html`<div class="item">
          <span class=${"chip" + (status ? " on" : "")}>${status || s.group}</span>
          <div><b>${s.title}</b><small>${s.summary}. <a href=${"https://github.com/" + s.source} target="_blank" rel="noreferrer">${s.source}</a></small></div>
          <div class="seg">
            ${!where.project && html`<button class="btn ghost" disabled=${!!busy} onClick=${() => install(s, false)}>${busy === s.name + false ? "…" : "Here"}</button>`}
            ${!where.global && html`<button class="btn ghost" disabled=${!!busy} onClick=${() => install(s, true)}>${busy === s.name + true ? "…" : "For me"}</button>`}
          </div>
        </div>`;
      })}`}
  </article>`;
}

// ---- running now ----
function Background({ toast }) {
  const [bg, setBg] = useState(null);
  const [armed, setArmed] = useState(false);
  const refresh = () => call("/api/background").then(setBg).catch(() => {});
  usePoll(refresh, 4000);
  const act = async (body) => {
    try { const r = await call("/api/background", body); toast(r.note); if (body.what !== "settings") refresh(); }
    catch (err) { toast(err.message, true); }
  };
  const head = html`<div class="head"><h2>Running now</h2><p>Quitting orcha closes the window. These keep going until you stop them.</p></div>`;
  if (!bg) return html`<article id="background">${head}<p class="muted">Looking…</p></article>`;
  const names = { blocked: "needs you", working: "working", done: "done", idle: "idle", unknown: "idle" };
  const rank = { blocked: 0, working: 1, done: 2 };
  const agents = [...bg.agents].sort((a, b) => (rank[a.status] ?? 3) - (rank[b.status] ?? 3));
  const counts = agents.reduce((m, a) => { const k = names[a.status] || a.status; m[k] = (m[k] || 0) + 1; return m; }, {});
  const chipFor = { "needs you": "blocked", working: "working", done: "done" };
  return html`<article id="background">${head}
    <div class="bgrow">
      <div><b>orcha's engine</b><small>${bg.server.running ? `version ${bg.server.version || ""}, ${agents.length} agents` : "not running"}</small></div>
      <span />
      ${bg.server.running && html`<button class="btn danger" onClick=${() => { if (armed) act({ what: "server" }); else { setArmed(true); setTimeout(() => setArmed(false), 4000); } }}>
        ${armed ? "Click again to stop all" : "Stop everything"}</button>`}
    </div>
    <div class="counts">${Object.entries(counts).map(([k, n]) => html`<span class=${"chip " + (chipFor[k] || "")}><span class="num">${n}</span> ${k}</span>`)}</div>
    ${agents.map((a) => html`<div class="bgrow">
      <div><b>${a.project || a.pane_id}</b><small>${a.agent || "agent"} in ${a.cwd}</small></div>
      <span class=${"chip " + (chipFor[names[a.status]] || "")}>${names[a.status] || a.status}</span>
      <button class="btn ghost" onClick=${() => act({ what: "focus", pane_id: a.pane_id })}>Go to</button>
    </div>`)}
    ${bg.plugin_panes.map((p) => html`<div class="bgrow">
      <div><b>${p.plugin_id}</b><small>plugin server in ${p.pane_id}</small></div>
      ${webUrl(p.url) ? html`<a class="btn ghost" role="button" href=${webUrl(p.url)} target="_blank" rel="noreferrer">Open</a>` : html`<span />`}
      <button class="btn ghost" onClick=${() => act({ what: "plugin_pane", pane_id: p.pane_id })}>Stop</button>
    </div>`)}
    ${bg.office && html`<div class="bgrow">
      <div><b>Easy Job</b><small>${bg.office.running ? "the office is open" + (bg.office.port ? ` on port ${bg.office.port}` : "") : "added, not running"}</small></div>
      <span class=${"chip" + (bg.office.running ? " on" : "")}>${bg.office.running ? "on" : "off"}</span>
      ${bg.office.running ? html`<button class="btn ghost" onClick=${() => act({ what: "office" })}>Stop</button>` : html`<span />`}
    </div>`}
    <div class="bgrow">
      <div><b>This page</b><small>it closes by itself a few minutes after the tab is gone</small></div><span />
      <button class="btn ghost" onClick=${() => act({ what: "settings" })}>Close now</button>
    </div>
  </article>`;
}

// ---- the page ----
function App() {
  const [state, setState] = useState(null);
  const [accounts, setAccounts] = useState(null);
  const [gone, setGone] = useState(false);
  const [mode, setMode] = useState(() => { try { return localStorage.getItem("orcha-settings-mode") || "dark"; } catch (_) { return "dark"; } });
  const [q, setQ] = useState("");
  const [note, setNote] = useState(null);
  const [active, setActive] = useState("look");
  const saving = useRef(false);
  const search = useRef(null);

  const toast = useCallback((text, bad, undo) => setNote({ text, bad, undo, at: Date.now() }), []);
  useEffect(() => {
    if (!note) return;
    const id = setTimeout(() => setNote(null), note.bad ? 4500 : note.undo ? 6000 : 2200);
    return () => clearTimeout(id);
  }, [note]);

  usePoll(async () => {
    if (saving.current) return;
    try { const next = await call("/api/state"); setState((old) => JSON.stringify(old) === JSON.stringify(next) ? old : next); setGone(false); }
    catch (_) { setGone(true); }
  }, 4000);
  usePoll(() => call("/api/accounts").then(setAccounts).catch(() => {}), 60000);

  const field = (key) => state?.sections.flatMap((s) => s.fields).find((f) => f.key === key);
  const save = useCallback(async (key, value, quiet) => {
    const before = field(key)?.value;
    saving.current = true;
    try {
      const data = await call("/api/set", { key, value });
      setState(data.state);
      const label = field(key)?.label || key;
      // saved, but a later step failed: the new state is shown, with why
      if (data.warning) toast(data.warning, true);
      else toast(data.note || (quiet ? `${label} is back` : `${label} saved`), false,
        quiet || before === undefined ? null : () => save(key, before, true));
    } catch (err) { toast(err.message, true); }
    saving.current = false;
  }, [state]);

  useEffect(() => {
    document.documentElement.dataset.theme = mode;
    try { localStorage.setItem("orcha-settings-mode", mode); } catch (_) {}
    const accent = field("accent");
    paintTheme(state?.page_theme?.[mode], accent?.options.find((o) => o.value === accent.value)?.color);
  }, [state, mode]);

  useEffect(() => {
    const onKey = (e) => {
      const typing = /INPUT|SELECT|TEXTAREA/.test(document.activeElement?.tagName || "");
      if (e.key === "/" && !typing) { e.preventDefault(); search.current?.focus(); }
      if (e.key === "Escape" && document.activeElement === search.current) { setQ(""); search.current.blur(); }
    };
    addEventListener("keydown", onKey);
    return () => removeEventListener("keydown", onKey);
  }, []);

  const ids = state ? [...state.sections.map((s) => s.id), "projects", "background"] : [];
  useEffect(() => {
    const onScroll = () => {
      let current = ids[0];
      for (const id of ids) {
        const node = document.getElementById(id);
        if (node && node.getBoundingClientRect().top < innerHeight * 0.3) current = id;
      }
      if (innerHeight + scrollY >= document.documentElement.scrollHeight - 4) current = ids[ids.length - 1];
      setActive(current);
    };
    addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => removeEventListener("scroll", onScroll);
  }, [ids.join()]);
  useEffect(() => { document.querySelector(`.tabs a[href="#${active}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" }); }, [active]);

  const jumped = useRef(false);
  useEffect(() => {
    if (!state || jumped.current) return;
    jumped.current = true;
    const target = location.hash.startsWith("#projects") ? "projects" : location.hash.slice(1);
    if (target) setTimeout(() => document.getElementById(target)?.scrollIntoView(), 50);
  }, [state]);

  const query = q.trim().toLowerCase();
  const badge = (s) => {
    const warns = s.fields.filter((f) => f.warn).length;
    if (warns) return html`<i class="warn" title="something to check">!</i>`;
    const n = s.fields.filter(changed).length;
    return n ? html`<i title=${n + " changed from the default"}>${n}</i>` : "";
  };
  const noHits = query && !state?.sections.some((s) => matches(query, s.title) || s.fields.some((f) => matches(query, f.label, f.help, f.key, f.group)));

  return html`
    <header class="top"><div class="bar">
      <div class="brand">orcha<span>settings</span></div>
      <input ref=${search} class="search" type="search" placeholder="Search settings   /" value=${q} onInput=${(e) => setQ(e.target.value)} aria-label="Search settings" />
      <span class="live">${gone ? "○ not connected" : html`<b>●</b> live`}</span>
      <button class="ghostbtn" title="switch light or dark" onClick=${() => setMode(mode === "dark" ? "light" : "dark")}>${mode === "dark" ? "Dark" : "Light"}</button>
    </div></header>
    ${!state ? html`<p class="gone">${gone ? "This page has closed. Open it again with: orcha settings" : "Loading…"}</p>` : html`
    <div class="page">
      <nav class="tabs">
        ${state.sections.map((s) => html`<a href=${"#" + s.id} class=${active === s.id ? "on" : ""}>${s.title}${badge(s)}</a>`)}
        <a href="#projects" class=${active === "projects" ? "on" : ""}>Projects</a>
        <a href="#background" class=${active === "background" ? "on" : ""}>Running now</a>
      </nav>
      <main>
        ${state.sections.map((s) => html`<${SettingsSection} key=${s.id} section=${s} state=${state} accounts=${accounts} setAccounts=${setAccounts} toast=${toast} save=${save} q=${query} />`)}
        <${Projects} toast=${toast} q=${query} />
        ${!query && html`<${Background} toast=${toast} />`}
        ${noHits && html`<p class="empty">Nothing matches “${q}”.</p>`}
      </main>
    </div>`}
    ${note && html`<div class=${"toast" + (note.bad ? " bad" : "")} key=${note.at}>${note.text}
      ${note.undo && html`<button onClick=${() => { const u = note.undo; setNote(null); u(); }}>Undo</button>`}</div>`}
  `;
}

render(html`<${App} />`, document.getElementById("app"));
