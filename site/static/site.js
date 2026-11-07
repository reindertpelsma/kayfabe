// Optional enhancements; every page works without this file.
// 1. Menu: below 48rem the header menu is a disclosure that starts closed; at and above it, it is
//    always open. Without JavaScript the list simply stays open.
// 2. Theme: a button cycles automatic / light / dark through the data-theme attribute and keeps the
//    choice in this browser only (localStorage, guarded: it can be missing or blocked).
// 3. A copy button on code blocks marked "copyable".
(() => {
  const menu = document.querySelector('details.menu');
  if (menu && window.matchMedia) {
    const narrow = window.matchMedia('(max-width: 47.99rem)');
    const sync = () => {
      menu.open = !narrow.matches;
    };
    sync();
    narrow.addEventListener('change', sync);
    menu.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && narrow.matches && menu.open) {
        menu.open = false;
        menu.querySelector('summary').focus();
      }
    });
  }

  const KEY = 'kf-theme';
  const root = document.documentElement;
  const read = () => {
    try {
      const value = localStorage.getItem(KEY);
      return value === 'light' || value === 'dark' ? value : '';
    } catch {
      return '';
    }
  };
  const write = (value) => {
    try {
      if (value) localStorage.setItem(KEY, value);
      else localStorage.removeItem(KEY);
    } catch {
      /* the choice then lasts for this page only */
    }
  };
  const apply = (value) => {
    if (value) root.setAttribute('data-theme', value);
    else root.removeAttribute('data-theme');
  };
  apply(read());
  const holder = document.querySelector('.site-header .bar');
  if (holder) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'theme-toggle';
    const label = () => {
      const now = root.getAttribute('data-theme') || 'auto';
      button.textContent = '\u25D0 ' + now;
      button.setAttribute('aria-label', 'Colour theme: ' + now + '. Activate to change.');
    };
    button.addEventListener('click', () => {
      const now = root.getAttribute('data-theme') || '';
      const next = now === '' ? 'light' : now === 'light' ? 'dark' : '';
      apply(next);
      write(next);
      label();
    });
    label();
    (holder.querySelector('.menu nav') ?? holder).append(button);
  }

  document.querySelectorAll('pre.copyable').forEach((pre) => {
    if (!navigator.clipboard) return;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'copy';
    button.textContent = 'Copy';
    button.setAttribute('aria-label', 'Copy the command to the clipboard');
    button.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(pre.innerText.trim());
        button.textContent = 'Copied';
      } catch {
        button.textContent = 'Copy failed';
      }
      setTimeout(() => {
        button.textContent = 'Copy';
      }, 2000);
    });
    pre.parentElement.appendChild(button);
  });
})();
