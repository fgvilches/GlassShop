// ═══════════════════════════════════════════════════════════════════════════════
// GlassShop — renderer logic. No frameworks; hand-rolled state + tiny helpers.
// ═══════════════════════════════════════════════════════════════════════════════

const $ = (sel, root) => (root || document).querySelector(sel);
const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));

const state = {
  view: 'home',
  categories: [],
  currentCategory: null,
  currentSub: null,
  titles: [],
  titlesTotal: null,
  titlesOffset: 0,
  sort: '', direction: 'desc',
  search: {
    q: '', qt: 'Text', results: [], total: null, offset: 0,
    filters: { include: [], exclude: [], type: 'none', sort: 'none', order: 'none' },
  },
  subcats: {},
  sheet: { open: false, title: null },
  pendingBaseFirst: null,
  updates: { available: [] },
  downloads: { items: [], settings: {}, active: 0 },
};

// ── formatting helpers ─────────────────────────────────────────────────────────
function fmtBytes(n) {
  if (n == null || isNaN(n)) return '—';
  const u = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let i = 0; let v = n;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)}\u00A0${u[i]}`;
}
function fmtSpeed(bps) { return bps > 0 ? `${fmtBytes(bps)}/s` : '—'; }
function fmtEta(s) {
  if (!s || s <= 0) return '—';
  if (s < 60) return `${Math.ceil(s)}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}
const div = document.createElement('div');
function esc(s) {
  div.textContent = String(s == null ? '' : s);
  return div.innerHTML;
}
function legitBadge(type) {
  const t = (type || '').toLowerCase();
  if (t.includes('legit')) return '<span class="badge b-legit">Legit</span>';
  if (t.includes('pirate')) return '<span class="badge b-pirate">Pirate Legit</span>';
  return '';
}

// ── toasts ────────────────────────────────────────────────────────────────────
function toast(msg, kind = '') {
  const el = document.createElement('div');
  el.className = `toast ${kind}`;
  el.textContent = msg;
  $('#toasts').appendChild(el);
  setTimeout(() => { el.style.opacity = '0'; el.style.transition = 'opacity 300ms'; setTimeout(() => el.remove(), 320); }, 4200);
}

// ── navigation ────────────────────────────────────────────────────────────────
function show(view) {
  state.view = view;
  for (const v of ['home', 'titles', 'search', 'downloads', 'settings']) {
    $(`#view-${v}`).classList.toggle('hidden', v !== (view === 'titles' ? 'home' : view) && !(view === 'titles' && v === 'titles'));
  }
  // simpler: hide all, show target
  $$('.view').forEach(v => v.classList.add('hidden'));
  const target = view === 'titles' ? '#view-titles' : `#view-${view}`;
  $(target).classList.remove('hidden');
  $$('#nav .sidebar-item').forEach(b => b.classList.toggle('active', b.dataset.view === view));

  if (view === 'downloads') renderDownloads();
  if (view === 'settings') renderSettings();
  if (view === 'search') {
    const input = $('#search-input');
    if (input && !input.value) input.focus();
  }
}

// ── categories sidebar ────────────────────────────────────────────────────────
async function loadCategories() {
  const cats = await window.hshop.categories();
  state.categories = cats;
  const nav = $('#cat-nav');
  nav.innerHTML = '';
  for (const c of cats) {
    const b = document.createElement('button');
    b.className = 'sidebar-item';
    b.innerHTML = `<span class="cat-dot"></span>${esc(c.name)}`;
    b.title = c.description || '';
    b.addEventListener('click', () => openCategory(c));
    nav.appendChild(b);
  }
}

// ── home: subcategory cards for current category (or category picker) ─────────
async function openCategory(c) {
  state.currentCategory = c;
  state.currentSub = null;
  $('#home-title').textContent = c.name;
  $('#home-sub').textContent = c.description || '';
  show('home');
  const grid = $('#subcat-grid');
  grid.innerHTML = '<div class="dl-empty">Cargando…</div>';
  try {
    const subs = await window.hshop.subcategories(c.id);
    if (!subs.length) { grid.innerHTML = '<div class="dl-empty">Sin subcategorías.</div>'; return; }
    grid.innerHTML = '';
    for (const s of subs) {
      const el = document.createElement('button');
      el.className = 'subcat-card glass';
      el.innerHTML = `
        <h3>${esc(s.name)}</h3>
        <p class="desc">${esc(s.description)}</p>
        <div class="subcat-stats">
          <span class="badge b-size">${esc(s.sizeText || '—')}</span>
          <span class="badge b-dl">${s.titles} títulos</span>
          <span class="badge b-legit">${s.official} oficiales</span>
          <span class="badge b-id">${fmtInt(s.downloads)} descargas</span>
        </div>`;
      el.addEventListener('click', () => openSubcategory(c, s));
      grid.appendChild(el);
    }
  } catch (e) {
    grid.innerHTML = `<div class="dl-empty">Error: ${esc(e.message || String(e))}</div>`;
  }
}

function fmtInt(n) { return (n || 0).toLocaleString('es'); }

// ── titles grid ────────────────────────────────────────────────────────────────
async function openSubcategory(cat, sub) {
  state.currentSub = sub;
  state.titles = []; state.titlesOffset = 0; state.titlesTotal = null;
  $('#titles-title').textContent = `${cat.name} › ${sub.name}`;
  $('#titles-sub').textContent = sub.description || '';
  show('titles');
  $('#titles-grid').innerHTML = '<div class="dl-empty">Cargando…</div>';
  await loadTitlesPage();
}

async function loadTitlesPage() {
  const cat = state.currentCategory, sub = state.currentSub;
  try {
    const res = await window.hshop.titles({
      category: cat.id, subcategory: sub.id,
      count: 50, offset: state.titlesOffset,
      sortBy: state.sort || undefined, direction: state.sort ? state.direction : undefined,
    });
    state.titles.push(...res.titles);
    if (state.titlesTotal == null) state.titlesTotal = res.total;
    state.titlesOffset += res.titles.length;
    renderTitles();
  } catch (e) {
    $('#titles-grid').innerHTML = `<div class="dl-empty">Error: ${esc(e.message || String(e))}</div>`;
  }
}

function renderTitles() {
  const grid = $('#titles-grid');
  if (!state.titles.length) { grid.innerHTML = '<div class="dl-empty">Sin resultados.</div>'; $('#load-more-wrap').classList.add('hidden'); return; }
  if (state.titlesOffset === state.titles.length) grid.innerHTML = ''; // full rerender
  else {
    // append only new rows on "load more"
  }
  const startIdx = grid.children.length && state.titlesOffset > 50 ? state.titles.length - (state.titlesOffset - (state.titlesTotal ? 0 : 0)) : 0;
  // simplest correct approach: rerender all (50–200 rows are cheap)
  grid.innerHTML = '';
  for (const t of state.titles) grid.appendChild(titleCard(t));
  const more = state.titlesTotal == null || state.titles.length < state.titlesTotal;
  $('#load-more-wrap').classList.toggle('hidden', !more);
}

function titleCard(t) {
  const el = document.createElement('button');
  el.className = 'title-card glass';
  el.innerHTML = `
    <h3>${esc(t.name)}</h3>
    <p class="tc-path">${esc((t.categoryPath || '').replace(/^content in /i, ''))}</p>
    <div class="tc-badges">
      ${legitBadge(t.contentType)}
      ${t.sizeText ? `<span class="badge b-size">${esc(t.sizeText)}</span>` : ''}
      ${t.titleId ? `<span class="badge b-tid">${esc(t.titleId)}</span>` : ''}
      ${t.productCode ? `<span class="badge b-prod">${esc(t.productCode)}</span>` : ''}
      <span class="badge b-id">#${esc(t.id)}</span>
    </div>
    <div class="tc-foot">
      <span class="badge b-dl">Descargar ↓</span>
    </div>`;
  el.addEventListener('click', () => openSheet(t));
  return el;
}

// ── search ────────────────────────────────────────────────────────────────────
async function doSearch(more = false) {
  const q = $('#search-input').value.trim();
  if (!q) { toast('Escribe algo para buscar.', 'err'); return; }
  state.search.q = q; state.search.qt = $('#search-qt').value;
  if (!more) { state.search.results = []; state.search.offset = 0; state.search.total = null; }
  $('#search-grid').innerHTML = state.search.results.length ? '' : '<div class="dl-empty">Buscando…</div>';
  try {
    const f = state.search.filters;
    const res = await window.hshop.search({
      query: q,
      qt: state.search.qt,
      count: 50,
      offset: state.search.offset,
      include: f.include.join(','),
      exclude: f.exclude.join(','),
      contentType: f.type,
      sortBy: f.sort,
      direction: f.order,
    });
    if (res && res.error) throw new Error(res.error);
    state.search.results.push(...res.titles);
    state.search.total = res.total;
    state.search.offset += res.titles.length;
    const g = $('#search-grid');
    g.innerHTML = '';
    for (const t of state.search.results) g.appendChild(titleCard(t));
    $('#search-summary').textContent = `${state.search.results.length} de ${state.search.total ?? '?'} resultados para “${q}”`;
    $('#search-summary').classList.remove('hidden');
    $('#search-more-wrap').classList.toggle('hidden', state.search.total == null || state.search.results.length >= state.search.total);
  } catch (e) {
    $('#search-grid').innerHTML = `<div class="dl-empty">Error: ${esc(e.message || String(e))}</div>`;
  }
}

// ── filtros de búsqueda ───────────────────────────────────────────────────────
function filterLabel(path) {
  const [cat, sub] = String(path).split('.');
  const c = (state.categories || []).find((x) => x.id === cat);
  const catName = c ? c.name : cat;
  if (!sub || sub === '*') return catName;
  const subs = state.subcats[cat] || [];
  const s = subs.find((x) => x.id === sub);
  return `${catName} › ${s ? s.name : sub}`;
}

function renderFilterChips() {
  const f = state.search.filters;
  const all = [
    ...f.include.map((p) => ['include', p]),
    ...f.exclude.map((p) => ['exclude', p]),
  ];

  const paint = (host, interactive) => {
    if (!host) return;
    host.innerHTML = '';
    host.classList.toggle('hidden', all.length === 0);
    for (const [mode, path] of all) {
      const chip = document.createElement('span');
      chip.className = `filter-chip${mode === 'exclude' ? ' exclude' : ''}`;
      const text = document.createElement('span');
      text.textContent = `${mode === 'exclude' ? '≠ ' : ''}${filterLabel(path)}`;
      chip.appendChild(text);
      if (interactive) {
        const x = document.createElement('button');
        x.type = 'button';
        x.textContent = '✕';
        x.setAttribute('aria-label', `Quitar filtro ${filterLabel(path)}`);
        x.addEventListener('click', () => {
          const list = f[mode];
          const i = list.indexOf(path);
          if (i >= 0) list.splice(i, 1);
          renderFilterChips();
        });
        chip.appendChild(x);
      }
      host.appendChild(chip);
    }
  };

  paint($('#filter-chips'), true);
  paint($('#active-chips'), false);

  const count = all.length;
  const badge = $('#filter-count');
  if (badge) {
    badge.textContent = String(count);
    badge.classList.toggle('hidden', count === 0);
  }
  const empty = $('#filter-empty');
  if (empty) empty.classList.toggle('hidden', count > 0);
}

async function fillSubcategories(catId) {
  const sel = $('#filter-sub');
  if (!sel) return;

  const keep = sel.value;
  sel.innerHTML = '<option value="none">Todas las regiones</option>';

  if (!catId || catId === 'none') {
    sel.disabled = true;
    return;
  }
  sel.disabled = false;

  if (!state.subcats[catId]) {
    sel.innerHTML = '<option value="none">Cargando regiones…</option>';
    let subs = null;
    try {
      subs = await window.hshop.subcategories(catId);
    } catch (e) {
      subs = { error: e.message || String(e) };
    }
    if (!Array.isArray(subs)) {
      const why = (subs && subs.error) || 'respuesta inesperada';
      sel.innerHTML = `<option value="none">No se pudieron cargar (${esc(String(why)).slice(0, 60)})</option>`;
      return;
    }
    state.subcats[catId] = subs;
  }

  sel.innerHTML = '<option value="none">Todas las regiones</option>';
  for (const s of state.subcats[catId]) {
    const o = document.createElement('option');
    o.value = s.id;
    o.textContent = s.name || s.id;
    sel.appendChild(o);
  }
  // a category with no subcategories still needs a usable state
  if (!state.subcats[catId].length) {
    sel.innerHTML = '<option value="none">Esta categoría no tiene regiones</option>';
  }
  if (keep && keep !== 'none') sel.value = keep;
}

async function openFilters() {
  const catSel = $('#filter-cat');
  if (catSel.options.length <= 1) {
    for (const c of state.categories || []) {
      const o = document.createElement('option');
      o.value = c.id;
      o.textContent = c.name;
      catSel.appendChild(o);
    }
  }
  if (state.currentCategory && state.currentCategory !== 'none' && catSel.value === 'none') {
    catSel.value = state.currentCategory;
  }
  // the region list only makes sense with a category chosen — default to games
  if (catSel.value === 'none') catSel.value = 'games';
  await fillSubcategories(catSel.value);

  const f = state.search.filters;
  $('#filter-type').value = f.type;
  $('#filter-sort').value = f.sort;
  $('#filter-order').value = f.order;
  renderFilterChips();

  $('#filters-sheet').classList.remove('hidden');
  $('#filters-scrim').classList.remove('hidden');
}

function closeFilters() {
  $('#filters-sheet').classList.add('hidden');
  $('#filters-scrim').classList.add('hidden');
}

function addFilterChip() {
  const cat = $('#filter-cat').value;
  const sub = $('#filter-sub').value;
  const mode = $('#filter-mode').value;
  if (cat === 'none') { toast('Elige una categoría.', 'err'); return; }
  const path = sub === 'none' ? `${cat}.*` : `${cat}.${sub}`;
  const f = state.search.filters;
  if (f.include.includes(path) || f.exclude.includes(path)) {
    toast('Ese filtro ya está aplicado.', 'err');
    return;
  }
  f[mode].push(path);
  renderFilterChips();
}

function applyFilters() {
  const f = state.search.filters;
  f.type = $('#filter-type').value;
  f.sort = $('#filter-sort').value;
  f.order = $('#filter-order').value;
  renderFilterChips();
  closeFilters();
  if ($('#search-input').value.trim()) doSearch();
}

function clearFilters() {
  state.search.filters = { include: [], exclude: [], type: 'none', sort: 'none', order: 'none' };
  $('#filter-type').value = 'none';
  $('#filter-sort').value = 'none';
  $('#filter-order').value = 'none';
  renderFilterChips();
}

// ── detail sheet ──────────────────────────────────────────────────────────────
async function openSheet(t) {
  state.sheet.open = true; state.sheet.title = t;
  $('#sheet-crumb').textContent = (t.categoryPath || '').replace(/^content in /i, '') || `Título #${t.id}`;
  $('#sheet-title').textContent = t.name;
  $('#sheet-meta').innerHTML = `
    <dt>Tamaño</dt><dd>${esc(t.sizeText || '—')}</dd>
    <dt>Title ID</dt><dd>${esc(t.titleId || '—')}</dd>
    <dt>Cód. producto</dt><dd>${esc(t.productCode || '—')}</dd>
    <dt>Tipo</dt><dd>${esc(t.contentType || '—')}</dd>`;
  $('#sheet-desc').textContent = '';
  $('#sheet-hash').textContent = '';
  $('#sheet-scrim').classList.remove('hidden');
  $('#sheet').classList.remove('hidden');
  // fetch full detail async
  try {
    const d = await window.hshop.titleDetail(t.id);
    state.sheet.detail = d;
    $('#sheet-meta').innerHTML = `
      <dt>Tamaño</dt><dd>${esc(d.sizeText || '—')}</dd>
      <dt>Title ID</dt><dd>${esc(d.titleIdText || d.titleId || '—')}</dd>
      <dt>Cód. producto</dt><dd>${esc(d.productCode || '—')}</dd>
      <dt>Tipo</dt><dd>${esc(d.contentType || '—')}</dd>
      <dt>Añadido</dt><dd>${esc(d.added || '—')}</dd>
      <dt>Actualizado</dt><dd>${esc(d.updated || '—')}</dd>
      <dt>Descargas</dt><dd>${esc(d.downloads || '—')}</dd>`;
    $('#sheet-desc').textContent = d.description || 'Sin descripción disponible.';
    $('#sheet-hash').textContent = d.sha256 ? `SHA-256: ${d.sha256}` : '';
  } catch { /* keep partial */ }
}

function closeSheet() {
  state.sheet.open = false;
  $('#sheet-scrim').classList.add('hidden');
  $('#sheet').classList.add('hidden');
}

// ── downloads ─────────────────────────────────────────────────────────────────
function downloadItemInfoFromSheet() {
  const t = state.sheet.title, d = state.sheet.detail || {};
  return {
    id: t.id, name: t.name,
    filename: `${t.name}.cia`,
    sizeBytes: d.sizeBytes || t.sizeBytes || null,
    sha256: d.sha256 || null,
    url: null,
  };
}

async function startDownloadFromSheet() {
  const t = state.sheet.title;
  if (!t) return;
  const btn = $('#sheet-download');
  const restore = () => {
    btn.disabled = false;
    btn.innerHTML = `<svg viewBox="0 0 16 16" width="16" height="16"><path fill="currentColor" d="M8 1a1 1 0 0 1 1 1v6.586l2.293-2.293a1 1 0 1 1 1.414 1.414l-4 4a1 1 0 0 1-1.414 0l-4-4a1 1 0 1 1 1.414-1.414L7 8.586V2a1 1 0 0 1 1-1M2 13h12v2H2z"/></svg> Descargar`;
  };

  // what actually queues the title, once base-first has had its say
  const run = async () => {
    btn.disabled = true;
    btn.textContent = 'Verificando…';
    try {
      const links = await window.hshop.resolveDownload(t.id);
      if (!links || links.error || !links.direct) {
        throw new Error((links && links.error) || 'no se obtuvo el enlace de descarga');
      }
      const info = downloadItemInfoFromSheet();
      info.url = links.direct;
      const res = await window.hshop.addDownload(info);
      if (res && res.error) throw new Error(res.error);
      toast(`Añadido a descargas: ${t.name}`, 'ok');
      closeSheet();
      show('downloads');
      // if hShop publishes an update or DLC for this game, offer them
      maybeExtras(pc, t.name);
    } catch (e) {
      toast(`No se pudo iniciar: ${e.message || e}`, 'err');
    } finally {
      restore();
    }
  };

  const detail = state.sheet.detail || {};
  const pc = detail.productCode || t.productCode || null;
  const isUpdate = typeof pc === 'string' && /^[A-Za-z]{3}-U-/i.test(pc);

  if (!isUpdate) {
    await run();
    return;
  }

  // an update: make sure its base game is there before queueing it
  btn.disabled = true;
  btn.textContent = 'Comprobando juego base…';
  try {
    await maybeBaseFirst({ id: t.id, name: t.name, productCode: pc }, run);
  } catch {
    await run();
  } finally {
    restore();
  }
}

// ── downloads view ───────────────────────────────────────────────────────────
function renderDownloads() {
  const { items, settings } = state.downloads;
  const list = $('#dl-list');
  const active = items.filter(i => i.status === 'downloading' || i.status === 'queued').length;
  $('#dl-badge').classList.toggle('hidden', active === 0);
  $('#dl-badge').textContent = active;
  $('#dl-summary').textContent = active
    ? `${active} en curso · ${items.filter(i => i.status === 'downloading').length} descargando`
    : (items.length ? `${items.length} en la lista` : 'Sin descargas activas.');

  if (!items.length) { list.innerHTML = '<div class="dl-empty">Aún no hay descargas.<br>Explora el catálogo y pulsa “Descargar”.</div>'; return; }
  list.innerHTML = '';
  for (const it of items) list.appendChild(dlRow(it));
}

function dlRow(it) {
  const el = document.createElement('div');
  el.className = 'dl-row glass';
  const pct = it.sizeBytes ? Math.min(100, (it.received / it.sizeBytes) * 100) : (it.status === 'completed' ? 100 : 0);
  const actions = [];
  if (it.status === 'downloading' || it.status === 'queued') actions.push(`<button class="glass-btn small" data-act="pause">Pausar</button>`);
  if (it.status === 'paused' || it.status === 'failed' || it.status === 'cancelled') actions.push(`<button class="glass-btn small" data-act="resume">Reanudar</button>`);
  if (it.status === 'failed') actions.push(`<button class="glass-btn small" data-act="retry">Reintentar</button>`);
  if (it.status === 'completed') {
    actions.push(`<button class="glass-btn small" data-act="reveal">Mostrar</button>`);
    if (it.verified === false) actions.push(`<span class="badge b-pirate">Hash no coincide</span>`);
    else if (it.verified === true) actions.push(`<span class="badge b-legit">Verificado ✓</span>`);
  }
  actions.push(`<button class="glass-btn small danger" data-act="remove">✕</button>`);

  el.innerHTML = `
    <div class="dl-info">
      <p class="dl-name" title="${esc(it.name)}">${esc(it.name)}</p>
      <p class="dl-meta">
        <span class="dl-status st-${it.status}">${statusLabel(it.status)}</span>
        <span>${fmtBytes(it.received)}${it.sizeBytes ? ` / ${fmtBytes(it.sizeBytes)}` : ''}</span>
        ${it.status === 'downloading' ? `<span>${fmtSpeed(it.speed)}</span><span>ETA ${fmtEta(it.eta)}</span>` : ''}
        ${it.error ? `<span style="color: var(--c-danger)">${esc(it.error)}</span>` : ''}
      </p>
    </div>
    <div class="progress-track">
      <div class="progress-fill ${it.status === 'completed' ? 'done' : it.status === 'failed' ? 'err' : ''} ${(!it.sizeBytes && it.status === 'downloading') ? 'indet' : ''}"
           style="width: ${pct}%"></div>
    </div>
    <div class="dl-actions">${actions.join('')}</div>`;

  el.addEventListener('click', (ev) => {
    const b = ev.target.closest('[data-act]');
    if (!b) return;
    const act = b.dataset.act;
    if (act === 'pause') window.hshop.pause(it.id);
    if (act === 'resume') window.hshop.resume(it.id);
    if (act === 'retry') window.hshop.retry(it.id);
    if (act === 'reveal') window.hshop.reveal(it.id);
    if (act === 'remove') window.hshop.remove(it.id);
  });
  return el;
}

function statusLabel(s) {
  return { queued: 'En cola', downloading: 'Descargando', paused: 'Pausada', completed: 'Completada', failed: 'Error', cancelled: 'Cancelada' }[s] || s;
}

// ── actualizaciones ──────────────────────────────────────────────────────────
function renderUpdates() {
  const panel = $('#updates-panel');
  const list = $('#updates-list');
  const { available } = state.updates;
  panel.classList.toggle('hidden', available.length === 0);
  const sub = $('#updates-sub');
  if (sub) {
    sub.textContent = available.length
      ? `${available.length} pendiente${available.length === 1 ? '' : 's'} para los juegos de tu biblioteca`
      : '';
  }
  if (!available.length) return;
  list.innerHTML = '';
  for (const u of available) {
    const el = document.createElement('div');
    el.className = 'update-row';
    el.innerHTML = `
      <div class="dl-info">
        <p class="dl-name">${esc(u.name)}</p>
        <p class="dl-meta">
          <span>${u.currentVersion ? `${esc(u.currentVersion)} → ` : 'sin update en disco → '}${esc(u.availableVersion)}</span>
        </p>
      </div>
      <div class="dl-actions">
        <button class="glass-btn small" data-act="enqueue-update" data-id="${esc(u.titleId)}">Agregar a la cola</button>
      </div>`;
    el.addEventListener('click', (ev) => {
      const b = ev.target.closest('[data-act]');
      if (!b) return;
      if (b.dataset.act === 'enqueue-update') startUpdateDownload(u);
    });
    list.appendChild(el);
  }
}

// ── contenido adicional (updates + DLC de la misma familia) ─────────────────
async function maybeExtras(productCode, gameName) {
  if (!productCode || !/^[A-Za-z]{3}-[A-Za-z]-/i.test(productCode)) return;
  if (/^[A-Za-z]{3}-U-/i.test(productCode)) return; // an update: no extras of itself

  let items = [];
  try {
    items = await window.hshop.updatesExtras(productCode);
  } catch {
    return;
  }
  if (!Array.isArray(items) || !items.length) return;

  state.pendingExtras = items;
  const updates = items.filter((i) => i.kind === 'update');
  const dlc = items.filter((i) => i.kind === 'dlc');

  $('#extras-title').textContent = `Contenido adicional de ${gameName}`;
  const bits = [];
  if (updates.length) bits.push(`${updates.length} actualizaci${updates.length === 1 ? 'ón' : 'ones'}`);
  if (dlc.length) bits.push(`${dlc.length} DLC`);
  $('#extras-msg').textContent =
    `hShop publica ${bits.join(' y ')} para la misma región. Elige qué quieres encolar junto al juego.`;

  $('#extras-list').innerHTML = items
    .map(
      (it, i) => `
      <label class="extras-item">
        <input type="checkbox" class="extras-check" data-idx="${i}" checked />
        <span class="extras-body">
          <span class="extras-name">${esc(it.name)}</span>
          <span class="extras-meta">
            <span class="extras-kind ${it.kind}">${it.kind === 'update' ? 'Update' : 'DLC'}</span>
            ${it.version ? `<span>v${esc(it.version)}</span>` : ''}
            ${it.sizeText ? `<span>${esc(it.sizeText)}</span>` : ''}
            ${it.productCode ? `<span class="mono">${esc(it.productCode)}</span>` : ''}
          </span>
        </span>
      </label>`,
    )
    .join('');

  $('#extras-scrim').classList.remove('hidden');
  $('#extras-sheet').classList.remove('hidden');
  syncExtrasButton();
}

function syncExtrasButton() {
  const btn = $('#extras-add');
  if (!btn) return;
  const n = document.querySelectorAll('.extras-check:checked').length;
  btn.disabled = n === 0;
  btn.textContent = n === 0 ? 'Nada seleccionado' : `Encolar ${n} seleccionado${n === 1 ? '' : 's'}`;
}

function closeExtras() {
  $('#extras-scrim').classList.add('hidden');
  $('#extras-sheet').classList.add('hidden');
  state.pendingExtras = null;
}

async function enqueueExtras() {
  const items = state.pendingExtras || [];
  const picks = [...document.querySelectorAll('.extras-check:checked')]
    .map((el) => items[Number(el.dataset.idx)])
    .filter(Boolean);
  closeExtras();
  if (!picks.length) return;

  let ok = 0;
  for (const it of picks) {
    try {
      const links = await window.hshop.resolveDownload(it.pageId);
      if (!links || links.error || !links.direct) {
        throw new Error((links && links.error) || 'sin enlace');
      }
      const res = await window.hshop.addDownload({
        id: it.pageId,
        name: it.name,
        filename: `${it.name}.cia`,
        url: links.direct,
        sizeBytes: it.sizeBytes || null,
      });
      if (res && res.error) throw new Error(res.error);
      ok += 1;
    } catch (e) {
      toast(`No se pudo agregar ${it.name}: ${e.message || e}`, 'err');
    }
  }
  if (ok) toast(`${ok} agregado${ok === 1 ? '' : 's'} a descargas`, 'ok');
}

// ── base-first (update sin juego base) ───────────────────────────────────────
// Shared by the updates panel and by a plain download of an update entry.
// `proceed` is what happens when there is nothing to warn about.
async function maybeBaseFirst(update, proceed) {
  let res = null;
  try {
    res = await window.hshop.updatesBaseFirst(update);
  } catch { /* offline or older bridge */ }
  if (!res || res.error) { await proceed(); return; }

  // the base game is already on disk
  if (res.base) {
    toast(`El juego base ya está en tu carpeta: ${res.base.name}`, 'ok');
    await proceed();
    return;
  }

  // not on disk and not on hShop: nothing to offer
  if (!res.baseTitle) {
    toast('No encontré el juego base en hShop: se agrega solo el update.', 'err');
    await proceed();
    return;
  }

  state.pendingBaseFirst = { base: res.baseTitle, update, proceed };
  $('#basefirst-title').textContent = `Update de ${res.baseTitle.name}`;
  $('#basefirst-msg').textContent =
    `No tienes el juego base de esta actualización («${esc(res.baseTitle.name)}»). ` +
    `Puedes descargar el juego primero y luego el update, o bajar solo el update ` +
    `(no sirve sin el juego instalado).`;
  $('#basefirst-scrim').classList.remove('hidden');
  $('#basefirst-sheet').classList.remove('hidden');
}

async function startUpdateDownload(u) {
  await maybeBaseFirst(
    { id: u.titleId, name: u.name, productCode: u.productCode || null },
    () => enqueueUpdate(u),
  );
}

async function enqueueUpdate(u) {
  // `titleId` is the 16-hex Title ID; downloads resolve from the page id
  const pageId = String(u.pageId || u.titleId);
  const btn = document.querySelector(`[data-id="${u.titleId}"]`);
  if (btn) { btn.disabled = true; btn.textContent = 'Resolviendo…'; }
  try {
    const links = await window.hshop.resolveDownload(pageId);
    if (!links || links.error || !links.direct) {
      throw new Error((links && links.error) || 'no se obtuvo el enlace');
    }
    const res = await window.hshop.addDownload({
      id: pageId, name: u.name, filename: `${u.name}.cia`,
      url: links.direct,
    });
    if (res && res.error) throw new Error(res.error);
    toast(`Añadido a descargas: ${u.name}`, 'ok');
    await window.hshop.updatesCheck();
  } catch (e) {
    toast(`No se pudo agregar: ${e.message || e}`, 'err');
  } finally {
    if (btn) { btn.disabled = false; btn.textContent = 'Agregar a la cola'; }
  }
}

async function enqueueBaseAndUpdate() {
  const p = state.pendingBaseFirst;
  if (!p) return;
  const proceed = p.proceed;
  closeBaseFirst();

  const baseId = String(p.base.id || p.base.titleId || '');
  if (baseId) {
    try {
      const links = await window.hshop.resolveDownload(baseId);
      if (!links || links.error || !links.direct) {
        throw new Error((links && links.error) || 'no se obtuvo el enlace');
      }
      const res = await window.hshop.addDownload({
        id: baseId,
        name: p.base.name,
        filename: `${p.base.name}.cia`,
        url: links.direct,
        sizeBytes: p.base.sizeBytes || null,
      });
      if (res && res.error) throw new Error(res.error);
      toast(`Añadido a descargas: ${p.base.name}`, 'ok');
    } catch (e) {
      toast(`No se pudo agregar el juego base: ${e.message || e}`, 'err');
    }
  }

  if (typeof proceed === 'function') await proceed();
}

function closeBaseFirst() {
  state.pendingBaseFirst = null;
  $('#basefirst-scrim').classList.add('hidden');
  $('#basefirst-sheet').classList.add('hidden');
}

// ── settings view ─────────────────────────────────────────────────────────────
// ── auto-update de la app ───────────────────────────────────────────────────
// The Rust side owns the real work (minisign check + bundle swap); this only
// drives the settings card and the progress bar.
async function initAppUpdate() {
  try {
    const cur = await window.hshop.updateCurrentVersion();
    if (cur && cur.version) {
      state.appUpdate = { current: cur.version };
      const el = $('#app-version');
      if (el) el.textContent = `Versión instalada: ${cur.version} · ${cur.platform}/${cur.arch}`;
    }
  } catch { /* older bridge */ }

  window.hshop.onUpdateProgress?.((p) => {
    const bar = $('#app-update-bar');
    if (!bar) return;
    bar.classList.remove('hidden');
    const fill = bar.querySelector('i');
    if (p && p.total) {
      const pct = Math.min(100, Math.round((p.chunk / p.total) * 100));
      if (fill) fill.style.width = `${pct}%`;
      const btn = $('#app-update-install');
      if (btn && btn.disabled) btn.textContent = `Descargando… ${pct}%`;
    } else if (fill) {
      fill.style.width = '30%';
      fill.classList.add('indet');
    }
  });

  // a quiet check a few seconds after launch — never blocks the UI
  setTimeout(() => {
    window.hshop
      .updateCheck()
      .then((res) => {
        if (res && res.available) markAppUpdate(res);
      })
      .catch(() => {});
  }, 5000);
}

function markAppUpdate(res) {
  state.appUpdate = { ...(state.appUpdate || {}), latest: res.version, notes: res.notes };
  const status = $('#app-update-status');
  if (status) status.textContent = `Versión ${res.version} disponible.`;
  const btn = $('#app-update-install');
  if (btn) {
    btn.classList.remove('hidden');
    btn.textContent = `Actualizar a ${res.version}`;
    btn.disabled = false;
  }
  document.querySelector('[data-view="settings"]')?.classList.add('has-update');
  toast(`GlassShop ${res.version} está disponible`, 'ok');
}

async function checkAppUpdate(btn) {
  const label = btn.textContent;
  btn.disabled = true;
  btn.textContent = 'Buscando…';
  try {
    const res = await window.hshop.updateCheck();
    if (res && res.error) throw new Error(res.error);
    if (res && res.available) {
      markAppUpdate(res);
    } else {
      const status = $('#app-update-status');
      if (status) status.textContent = `Estás al día (${(res && res.currentVersion) || '—'}).`;
      toast('GlassShop está actualizado', 'ok');
    }
  } catch (e) {
    const status = $('#app-update-status');
    if (status) status.textContent = 'No se pudo consultar el servidor de actualizaciones.';
    toast(`No se pudo consultar: ${e.message || e}`, 'err');
  } finally {
    btn.disabled = false;
    btn.textContent = label;
  }
}

async function installAppUpdate(btn) {
  btn.disabled = true;
  btn.textContent = 'Descargando… 0%';
  try {
    const res = await window.hshop.updateInstall();
    if (res && res.error) throw new Error(res.error);
    if (!res || !res.installed) {
      btn.disabled = false;
      btn.textContent = 'Actualizar';
      toast('No había nada que instalar', 'err');
      return;
    }
    btn.textContent = 'Reiniciando…';
    await window.hshop.updateRestart();
  } catch (e) {
    btn.disabled = false;
    btn.textContent = 'Reintentar';
    const status = $('#app-update-status');
    if (status) status.textContent = `Falló la actualización: ${e.message || e}`;
    toast(`No se pudo actualizar: ${e.message || e}`, 'err');
  }
}

function renderSettings() {
  const s = state.downloads.settings;
  if (!s || !s.downloadDir) return;
  $('#dl-dir-label').textContent = s.downloadDir;
  $('#concurrency').value = s.concurrency || 3;
  $('#concurrency-val').textContent = s.concurrency || 3;
  const lim = s.speedLimit || 0;
  $('#speed-limit').value = Math.min(50, lim ? Math.round(lim / 1048576) : 0);
  $('#speed-limit-val').textContent = lim ? `${Math.round(lim / 1048576)} MiB/s` : '∞';
  $('#toggle-verify').textContent = s.autoVerify ? 'Activada ✓' : 'Desactivada';
  renderLibrary();
}

// library = what the update checker knows about, from the folder on disk
async function renderLibrary() {
  const label = $('#library-summary');
  if (!label) return;
  try {
    const snap = await window.hshop.updatesSnapshot();
    if (!snap || snap.error) { label.textContent = '—'; return; }
    const indexed = (snap.installed || []).length;
    const missing = (snap.missing || []).length;
    const unindexed = (snap.unindexed || []).length;
    const parts = [`${indexed} en la biblioteca`];
    if (missing) parts.push(`${missing} ya no están en disco`);
    if (unindexed) parts.push(`${unindexed} sin indexar en la carpeta`);
    label.textContent = parts.join(' · ');
    const btn = $('#scan-folder');
    if (btn) btn.disabled = unindexed === 0;
  } catch {
    label.textContent = '—';
  }
}

async function scanFolder() {
  const btn = $('#scan-folder');
  const label = $('#library-summary');
  if (btn) { btn.disabled = true; btn.textContent = 'Escaneando…'; }
  try {
    const res = await window.hshop.updatesScan();
    if (res && res.error) throw new Error(res.error);
    const matched = res?.scan?.matched ?? 0;
    const skipped = (res?.scan?.skipped || []).length;
    toast(
      matched
        ? `Indexados ${matched} archivo(s)${skipped ? ` · ${skipped} sin coincidencia` : ''}`
        : 'No se pudo emparejar ningún archivo con el catálogo.',
      matched ? 'ok' : 'err',
    );
    if (res?.updates?.length) {
      state.updates.available = res.updates;
      renderUpdates();
    }
    await renderLibrary();
  } catch (e) {
    toast(`Error al escanear: ${e.message || e}`, 'err');
  } finally {
    if (btn) { btn.disabled = false; btn.textContent = 'Escanear carpeta'; }
  }
}

// ── wiring ─────────────────────────────────────────────────────────────────────
function wire() {
  // sidebar navigation
  $$('#nav .sidebar-item').forEach(b => b.addEventListener('click', () => show(b.dataset.view)));
  $('#open-site').addEventListener('click', () => window.hshop.openExternal('https://hshop.erista.me/'));

  // titles view
  $('#crumb-back').addEventListener('click', () => openCategory(state.currentCategory));
  $('#load-more').addEventListener('click', loadTitlesPage);
  $('#sort-sel').addEventListener('change', () => {
    state.sort = $('#sort-sel').value;
    state.titles = []; state.titlesOffset = 0;
    if (state.currentSub) loadTitlesPage();
  });
  $('#dir-sel').addEventListener('change', () => {
    state.direction = $('#dir-sel').value;
    state.titles = []; state.titlesOffset = 0;
    if (state.currentSub) loadTitlesPage();
  });

  // search
  $('#search-form').addEventListener('submit', (ev) => { ev.preventDefault(); doSearch(); });
  $('#search-more').addEventListener('click', () => doSearch(true));

  // filters modal
  $('#open-filters')?.addEventListener('click', openFilters);
  $('#filters-close')?.addEventListener('click', closeFilters);
  $('#filters-scrim')?.addEventListener('click', closeFilters);
  $('#filter-cat')?.addEventListener('change', (e) => fillSubcategories(e.target.value));
  $('#filter-add')?.addEventListener('click', addFilterChip);
  $('#filter-clear')?.addEventListener('click', clearFilters);
  $('#filter-apply')?.addEventListener('click', applyFilters);

  // sheet
  $('#sheet-close').addEventListener('click', closeSheet);
  $('#sheet-scrim').addEventListener('click', closeSheet);
  $('#sheet-open-site').addEventListener('click', () => {
    if (state.sheet.title) window.hshop.openExternal(state.sheet.title.url);
  });
  $('#sheet-download').addEventListener('click', startDownloadFromSheet);
  document.addEventListener('keydown', (ev) => {
    if (ev.key !== 'Escape') return;
    if (!$('#filters-sheet')?.classList.contains('hidden')) { closeFilters(); return; }
    if (state.sheet.open) closeSheet();
  });

  // downloads toolbar
  $('#pause-all').addEventListener('click', () => window.hshop.pauseAll());
  $('#resume-all').addEventListener('click', () => window.hshop.resumeAll());
  $('#clear-completed').addEventListener('click', () => window.hshop.clearCompleted());
  $('#open-dl-folder')?.addEventListener('click', async () => {
    const res = await window.hshop.openDownloadsFolder();
    if (res && res.error) toast(`No se pudo abrir la carpeta: ${res.error}`, 'err');
  });
  $('#check-updates').addEventListener('click', async (ev) => {
    const btn = ev.currentTarget;
    const restore = btn.textContent;
    btn.disabled = true;
    btn.textContent = 'Buscando…';
    try {
      const list = await window.hshop.updatesCheck();
      state.updates.available = Array.isArray(list) ? list : [];
      renderUpdates();
      const n = state.updates.available.length;
      toast(
        n ? `${n} actualizaci${n === 1 ? 'ón' : 'ones'} disponible${n === 1 ? '' : 's'}`
          : 'No hay actualizaciones nuevas para tu biblioteca',
        'ok',
      );
    } catch (e) {
      toast(`No se pudieron buscar actualizaciones: ${e.message || e}`, 'err');
    } finally {
      btn.disabled = false;
      btn.textContent = restore;
    }
  });

  // extras modal (updates + DLC del mismo juego)
  $('#extras-close')?.addEventListener('click', closeExtras);
  $('#extras-skip')?.addEventListener('click', closeExtras);
  $('#extras-scrim')?.addEventListener('click', closeExtras);
  $('#extras-list')?.addEventListener('change', syncExtrasButton);
  $('#extras-add')?.addEventListener('click', enqueueExtras);

  // base-first modal
  $('#basefirst-close').addEventListener('click', closeBaseFirst);
  $('#basefirst-scrim').addEventListener('click', closeBaseFirst);
  $('#basefirst-skip').addEventListener('click', async () => {
    const p = state.pendingBaseFirst;
    closeBaseFirst();
    if (p && typeof p.proceed === 'function') await p.proceed();
  });
  $('#basefirst-base-first').addEventListener('click', enqueueBaseAndUpdate);
  document.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape') { closeSheet(); closeBaseFirst(); }
  });

  // settings
  $('#pick-dir').addEventListener('click', async () => {
    const dir = await ipcPickDir();
    if (dir) window.hshop.setDownloadSettings({ downloadDir: dir });
  });
  $('#scan-folder')?.addEventListener('click', scanFolder);
  $('#app-update-check')?.addEventListener('click', (ev) => checkAppUpdate(ev.currentTarget));
  $('#app-update-install')?.addEventListener('click', (ev) => installAppUpdate(ev.currentTarget));
  $('#concurrency').addEventListener('input', () => {
    $('#concurrency-val').textContent = $('#concurrency').value;
    window.hshop.setDownloadSettings({ concurrency: +$('#concurrency').value });
  });
  $('#speed-limit').addEventListener('input', () => {
    const v = +$('#speed-limit').value;
    $('#speed-limit-val').textContent = v ? `${v} MiB/s` : '∞';
    window.hshop.setDownloadSettings({ speedLimit: v * 1048576 });
  });
  $('#toggle-verify').addEventListener('click', () => {
    const cur = state.downloads.settings.autoVerify;
    window.hshop.setDownloadSettings({ autoVerify: !cur });
    $('#toggle-verify').textContent = !cur ? 'Activada ✓' : 'Desactivada';
  });

  // live updates from main
  window.hshop.onDownloadsChanged((snap) => {
    state.downloads = snap;
    if (state.view === 'downloads') renderDownloads();
    else { // update badge only
      const active = snap.items.filter(i => i.status === 'downloading' || i.status === 'queued').length;
      $('#dl-badge').classList.toggle('hidden', active === 0);
      $('#dl-badge').textContent = active;
    }
    if (state.view === 'settings') renderSettings();
  });

  window.hshop.onUpdatesAvailable((payload) => {
    state.updates.available = payload.updates || [];
    renderUpdates();
  });

  window.hshop.onUpdatesBaseFirst((payload) => {
    startUpdateDownload(payload.update);
  });

  window.hshop.onNavigate(({ view }) => show(view));
}

// folder picker via IPC (main shows the native dialog)
async function ipcPickDir() {
  return window.hshop.pickDir ? await window.hshop.pickDir() : null;
}

// ── borderless window chrome (Windows / Linux) ────────────────────────────────
async function setupWindowChrome() {
  let os = 'macos';
  try {
    os = await window.hshop.platform();
  } catch { /* older bridge */ }
  if (os === 'macos') return;

  document.body.classList.add('borderless');
  const bar = $('#win-controls');
  if (bar) bar.classList.remove('hidden');

  // drag anywhere on a page header's empty space
  document.querySelectorAll('.page-head, .sidebar-header').forEach((el) =>
    el.setAttribute('data-tauri-drag-region', ''));

  $('#win-min')?.addEventListener('click', () => window.hshop.minimize());
  $('#win-max')?.addEventListener('click', () => window.hshop.toggleMaximize());
  $('#win-close')?.addEventListener('click', () => window.hshop.closeWindow());
}

// ── boot ───────────────────────────────────────────────────────────────────────
async function boot() {
  wire();
  await setupWindowChrome();
  state.updates = { available: [] };
  // expose a small dir picker through preload if available
  try {
    const snap = await window.hshop.snapshot();
    state.downloads = snap;
  } catch { }
  await loadCategories();
  await window.hshop.updatesCheck();
  show('home');
  initAppUpdate();
}
boot();
