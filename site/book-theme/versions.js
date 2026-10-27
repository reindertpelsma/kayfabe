const select = document.getElementById('doc-version');
fetch('/docs/versions.json')
  .then((response) => response.json())
  .then((versions) => {
    select.replaceChildren(...versions.map((v) => new Option(v.label, v.version)));
    select.value = select.dataset.current;
    select.onchange = async () => {
      const current = '/docs/' + encodeURIComponent(select.dataset.current) + '/';
      const base = '/docs/' + encodeURIComponent(select.value) + '/';
      const target = base + location.pathname.slice(current.length);
      try {
        location.href = (await fetch(target, { method: 'HEAD' })).ok ? target : base;
      } catch {
        location.href = base;
      }
    };
  })
  .catch(() => {
    select.hidden = true;
  });
