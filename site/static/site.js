// Optional enhancement: a copy button on code blocks marked "copyable". The page works without it.
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
