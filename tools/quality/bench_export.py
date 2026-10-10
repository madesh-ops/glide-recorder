"""End-to-end export quality: recorded (lossy) video -> real export -> compare.

Cases (static frame at t=1.0 s, plus a scroll frame at t=3.0 s):
  1to1   padding 0, zoom 1, 1080p : exact 1:1 mapping -> PSNR vs reference
  zoom2  padding 0, zoom 2 centred : vs lanczos x2 of the reference centre crop
  pad08  padding 8 %, 1080p        : vs lanczos downscale of the reference to the rect
Also reports a sharpness index (mean squared Laplacian of luma) relative to the ideal.
"""
import json, os, re, subprocess, sys

Q, ROOT = sys.argv[1], sys.argv[2]
ENC = sys.argv[3] if len(sys.argv) > 3 else os.path.join(Q, "enc_current_amf.mp4")
TAG = sys.argv[4] if len(sys.argv) > 4 else "current"
REF = os.path.join(Q, "reference.mkv")
EX = os.path.join(ROOT, "target", "release", "examples", "export_quality" + (".exe" if os.name == "nt" else ""))

def frame(path, t, out, vf=None):
    cmd = ["ffmpeg", "-v", "error", "-y", "-ss", str(t), "-i", path, "-frames:v", "1"]
    vf_chain = "scale=in_color_matrix=bt709:in_range=tv:flags=bicubic+accurate_rnd+full_chroma_int,format=rgb24"
    if vf:
        vf_chain += "," + vf
    if path.endswith(".mkv"):  # lossless RGB reference
        vf_chain = "format=rgb24" + ("," + vf if vf else "")
    subprocess.run(cmd + ["-vf", vf_chain, out], check=True)

def compare(a, b):
    r = subprocess.run(["ffmpeg", "-hide_banner", "-i", a, "-i", b, "-lavfi", "[0]format=gbrp[x];[1]format=gbrp[y];[x][y]psnr;[0]format=gbrp[p];[1]format=gbrp[q];[p][q]ssim", "-f", "null", "-"], capture_output=True, text=True)
    psnr = re.search(r"PSNR .*average:([0-9.inf]+)", r.stderr).group(1)
    ssim = re.search(r"SSIM .*All:([0-9.]+)", r.stderr).group(1)
    return float(psnr), float(ssim)

def sharp(path):
    # Mean of squared Laplacian on luma (higher = crisper edges).
    r = subprocess.run(["ffmpeg", "-hide_banner", "-i", path, "-vf", "format=gray,convolution='0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0',signalstats", "-f", "null", "-"], capture_output=True, text=True)
    # signalstats gives YAVG of |laplacian| (convolution output is clipped to >=0 so use as proxy).
    m = re.findall(r"YAVG:([0-9.]+)", r.stderr)
    if not m:
        r = subprocess.run(["ffprobe", "-v", "error", "-f", "lavfi", "-i", f"movie='{path.replace(chr(92), '/').replace(':', chr(92) + ':')}',format=gray,convolution='0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0:0 1 0 1 -4 1 0 1 0',signalstats", "-show_entries", "frame_tags=lavfi.signalstats.YAVG", "-of", "csv=p=0"], capture_output=True, text=True)
        return float(r.stdout.strip().splitlines()[0])
    return float(m[0])

cases = {
    "1to1": dict(padding=0, zoom=1, height=1080),
    "zoom2": dict(padding=0, zoom=2, height=1080),
    "pad08": dict(padding=0.08, zoom=1, height=1080),
    "pad08_1440": dict(padding=0.08, zoom=1, height=1440),
}
for name, c in cases.items():
    work = os.path.join(Q, f"exp_{TAG}_{name}")
    r = subprocess.run([EX, ENC, work, "--padding", str(c["padding"]), "--zoom", str(c["zoom"]), "--height", str(c["height"])], capture_output=True, text=True)
    if r.returncode != 0:
        print(name, "FAILED", r.stderr[-400:])
        continue
    g = json.loads(r.stdout.strip().splitlines()[-1])
    for t in (1.0, 3.0):
        got, ideal = os.path.join(work, f"got_{t}.png"), os.path.join(work, f"ideal_{t}.png")
        if name == "1to1":
            frame(os.path.join(work, "export.mp4"), t, got)
            frame(REF, t, ideal)
        elif name == "zoom2":
            frame(os.path.join(work, "export.mp4"), t, got)
            frame(REF, t, ideal, "crop=960:540:480:270,scale=1920:1080:flags=lanczos")
        else:
            x, y, w, h = g["rect"]
            ix, iy = int(round(x)) + 8, int(round(y)) + 8
            iw, ih = int(w) - 16, int(h) - 16
            frame(os.path.join(work, "export.mp4"), t, got, f"crop={iw}:{ih}:{ix}:{iy}")
            frame(REF, t, ideal, f"scale={round(w)}:{round(h)}:flags=lanczos,crop={iw}:{ih}:8:8")
        psnr, ssim = compare(got, ideal)
        sg, si = sharp(got), sharp(ideal)
        print(f"{TAG:10s} {name:6s} t={t}  PSNR {psnr:6.2f} dB  SSIM {ssim:.4f}  sharpness {sg / si * 100:5.1f}% of ideal   ({g['bytes']*8/6/1e6:.1f} Mbit/s, {g['secs']:.1f}s)")
