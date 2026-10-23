"""Generate a Dock-sized ICNS from the full-bleed app PNG (requires Pillow)."""
import argparse
import io
import struct
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('source', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--preview', type=Path)
parser.add_argument('--glyph', type=Path, default=Path(__file__).resolve().parent.parent / 'resources/icons/source.png')
parser.add_argument('--png', type=Path)
args = parser.parse_args()

original = Image.open(args.source).convert('RGBA').resize((1024, 1024), Image.Resampling.LANCZOS)
previous = Image.open(args.output).convert('RGBA') if args.output.exists() else original
# Redraw the background separately so optical centering moves only the mark.
glyph = Image.open(args.glyph).convert('RGBA')
glyph = glyph.crop(glyph.getchannel('A').getbbox())
canvas = Image.new('RGBA', (2048, 2048))
ImageDraw.Draw(canvas).rounded_rectangle((0, 0, 2047, 2047), radius=430, fill='#E8EEDF')
ratio = 1464 / max(glyph.size)
glyph = glyph.resize((round(glyph.width * ratio), round(glyph.height * ratio)), Image.Resampling.LANCZOS)
# The straight left stem carries more visual weight; shift right by 2% of the
# tile width. A 71.5% mark height adds a little more space above and below.
canvas.alpha_composite(glyph, ((2048-glyph.width)//2 + 40, (2048-glyph.height)//2))
original = canvas.resize((1024, 1024), Image.Resampling.LANCZOS)
# macOS does not add the transparent margin to a custom ICNS. Keep the visible
# rounded square inside 824x824 pixels, centered in the 1024x1024 canvas.
master = Image.new('RGBA', (1024, 1024))
master.alpha_composite(original.resize((824, 824), Image.Resampling.LANCZOS), (100, 100))
if args.png:
    args.png.parent.mkdir(parents=True, exist_ok=True)
    master.save(args.png)
chunks = []
for kind, size in [('icp4',16), ('icp5',32), ('icp6',64), ('ic07',128),
                   ('ic08',256), ('ic09',512), ('ic10',1024), ('ic11',32),
                   ('ic12',64), ('ic13',256), ('ic14',512)]:
    buf = io.BytesIO()
    master.resize((size, size), Image.Resampling.LANCZOS).save(buf, format='PNG')
    data = buf.getvalue()
    chunks.append(kind.encode() + struct.pack('>I', len(data) + 8) + data)
payload = b''.join(chunks)
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_bytes(b'icns' + struct.pack('>I', len(payload) + 8) + payload)
# Decode the actual container before accepting the asset.
with Image.open(args.output) as image:
    image.load()
    assert image.size == (1024, 1024)
    assert image.convert('RGBA').getchannel('A').getbbox() == (100, 100, 924, 924)
if args.preview:
    preview = Image.new('RGB', (660, 230), '#E5E7E6')
    draw = ImageDraw.Draw(preview)
    font = ImageFont.truetype('/System/Library/Fonts/Supplemental/Arial.ttf', 18)
    for x, icon, title in [(65, previous, 'Previous'), (395, master, 'Adjusted')]:
        scaled = icon.resize((140, 140), Image.Resampling.LANCZOS)
        preview.paste(scaled, (x, 24), scaled)
        draw.text((x, 186), title, font=font, fill='#1E302E')
    args.preview.parent.mkdir(parents=True, exist_ok=True)
    preview.save(args.preview)
print('macOS ICNS verified: 824px artwork / 1024px canvas, 100px transparent margin')
