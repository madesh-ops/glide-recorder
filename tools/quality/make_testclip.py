"""Build a lossless synthetic 'screen content' reference clip (1920x1080@60, 6 s).

Tall page with small UI text (light + dark panels, coloured code, 1 px rules),
shown static for 2 s, scrolling for 2 s, static again for 2 s.
"""
import os, subprocess, sys

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
W, H, PAGE_H = 1920, 1080, 2400

words = ("glide record zoom cursor export timeline segment render frame encoder bitrate quality "
         "sharp text pixel display window audio project settings preview camera smooth click").split()
def line(i, n):
    return " ".join(words[(i * 7 + k * 3) % len(words)] for k in range(n))

def esc(p):
    return p.replace("\\", "/").replace(":", "\\:")

seg = esc("C:/Windows/Fonts/segoeui.ttf")
mono = esc("C:/Windows/Fonts/consola.ttf")
filters = []
# Light panel (left 1100 px): UI text in several small sizes.
y = 20
for i in range(120):
    size = [12, 13, 14, 16][i % 4]
    tf = os.path.join(out, f"l{i}.txt")
    open(tf, "w").write(f"{i:03d}  " + line(i, 14).capitalize() + ".")
    filters.append(f"drawtext=fontfile='{seg}':textfile='{esc(tf)}':fontsize={size}:fontcolor=0x202124:x=24:y={y}")
    y += size + 6
    if y > PAGE_H - 30:
        break
# 1 px rules / table grid on the light panel.
for gy in range(0, PAGE_H, 96):
    filters.append(f"drawbox=x=16:y={gy}:w=1060:h=1:color=0xD0D0D8:t=fill")
# Dark panel (right): coloured monospace "code".
colors = ["0xC678DD", "0x61AFEF", "0x98C379", "0xE5C07B", "0xE06C75", "0xABB2BF"]
filters.insert(0, "drawbox=x=1100:y=0:w=820:h=%d:color=0x1E1F24:t=fill" % PAGE_H)
y = 20
i = 0
while y < PAGE_H - 30:
    tf = os.path.join(out, f"c{i}.txt")
    open(tf, "w").write(("    " * (i % 4)) + f"let {words[i % len(words)]}_{i} = {words[(i*5) % len(words)]}(x, {i});")
    filters.append(f"drawtext=fontfile='{mono}':textfile='{esc(tf)}':fontsize=13:fontcolor={colors[i % len(colors)]}:x=1120:y={y}")
    y += 19
    i += 1

page = os.path.join(out, "page.png")
script = os.path.join(out, "page_filters.txt")
open(script, "w").write(",\n".join(filters))
subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", f"color=c=0xF7F7F8:s={W}x{PAGE_H}",
                "-frames:v", "1", "-/vf", script, page], check=True)
# Static 2 s, scroll 600 px over 2 s, static 2 s.
crop_y = "if(lt(t,2),0,if(lt(t,4),(t-2)*300,600))"
subprocess.run(["ffmpeg", "-v", "error", "-y", "-loop", "1", "-framerate", "60", "-i", page, "-t", "6",
                "-vf", f"crop={W}:{H}:0:'{crop_y}'", "-c:v", "libx264rgb", "-qp", "0", "-preset", "ultrafast",
                os.path.join(out, "reference.mkv")], check=True)
print("ok")
