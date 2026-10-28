// Resolve from this script, not the origin root or the current chapter's depth.
const select = document.getElementById('doc-version');
const scriptURL = new URL(document.currentScript.src);
const currentRoot = new URL('../', scriptURL);
const docsRoot = new URL('../'.repeat(select.dataset.current.split('/').length), currentRoot);
const offline = location.protocol === 'file:';
select.value = select.dataset.current;
if (offline) select.title = 'Offline version changes open the version index';
select.onchange = async () => {
  const base = new URL(select.value.split('/').map(encodeURIComponent).join('/') + '/', docsRoot);
  const index = new URL('index.html', base).href;
  if (offline) {
    location.href = index;
    return;
  }
  const chapter = location.pathname.startsWith(currentRoot.pathname)
    ? location.pathname.slice(currentRoot.pathname.length) || 'index.html'
    : 'index.html';
  const target = new URL(chapter, base).href;
  try {
    location.href = (await fetch(target, { method: 'HEAD' })).ok ? target : index;
  } catch {
    location.href = index;
  }
};
// The build supplies options for file:// and for an unavailable HTTP JSON response.
if (!offline) {
  fetch(new URL('versions.json', docsRoot).href)
    .then((response) => {
      if (!response.ok) throw new Error('Version list unavailable');
      return response.json();
    })
    .then((versions) => {
      select.replaceChildren(...versions.map((v) => new Option(v.label, v.version)));
      select.value = select.dataset.current;
    })
    .catch(() => {
      select.title = 'Using the version list included in this document';
    });
}
