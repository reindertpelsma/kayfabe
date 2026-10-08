"""frames.py SHOTDIR OUT_PREFIX — classify the timestamped QMP screendumps of one boot (s-HHMMSS.mmm.ppm, host UTC)
and write OUT_PREFIX.tsv (time, mean, dark fraction, bottom-strip mean = the taskbar row, changed-vs-previous, class) and
OUT_PREFIX-sheet.png (a contact sheet of the frames where the class changes, 384x216 each, labelled with the time).
Classes: black (mean < 3), content (anything else). Needs PIL. Reads only the given directory."""
import os, sys
from PIL import Image, ImageDraw

d, out = sys.argv[1], sys.argv[2]
names = sorted(n for n in os.listdir(d) if n.startswith('s-') and n.endswith('.ppm'))
rows, prev, keys = [], None, []
for n in names:
    try:
        im = Image.open(os.path.join(d, n)).convert('L')
    except Exception as e:
        rows.append((n[2:-4], 'unreadable', str(e)))
        continue
    small = im.resize((192, 108))
    px = list(small.getdata())
    mean = sum(px) / len(px)
    dark = sum(1 for p in px if p < 8) / len(px)
    strip = list(small.crop((0, 100, 192, 108)).getdata())
    smean = sum(strip) / len(strip)
    changed = None if prev is None else sum(abs(a - b) for a, b in zip(px, prev)) / len(px)
    cls = 'black' if mean < 3 else 'content'
    if not keys or keys[-1][1] != cls or (changed is not None and changed > 20):
        keys.append((n, cls))
    prev = px
    rows.append((n[2:-4], '%.1f' % mean, '%.3f' % dark, '%.1f' % smean, '-' if changed is None else '%.1f' % changed, cls))
with open(out + '.tsv', 'w') as f:
    f.write('# utc\tmean\tdark_fraction\tbottom_strip_mean\tchange_vs_previous\tclass\n')
    for r in rows:
        f.write('\t'.join(r) + '\n')
keys = keys[:24]
if keys:
    w, h = 384, 216
    sheet = Image.new('RGB', (w * 4, (h + 16) * ((len(keys) + 3) // 4)), 'white')
    for i, (n, cls) in enumerate(keys):
        im = Image.open(os.path.join(d, n)).convert('RGB').resize((w, h))
        x, y = (i % 4) * w, (i // 4) * (h + 16)
        sheet.paste(im, (x, y + 16))
        ImageDraw.Draw(sheet).text((x + 4, y + 2), '%s UTC  %s' % (n[2:-4], cls), fill='black')
    sheet.save(out + '-sheet.png', optimize=True)
print(len(rows), 'frames;', len(keys), 'key frames')
