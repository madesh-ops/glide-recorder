"""Benchmark recording-encode configurations on the reference clip.

Mirrors the app: raw BGRA frames on stdin -> ffmpeg [vf + codec args] -> MP4.
Reports SSIM / PSNR in RGB (chroma losses count), Y-SSIM, bitrate and speed.
"""
import json, os, re, subprocess, sys, time

Q = sys.argv[1]
REF = os.path.join(Q, "reference.mkv")
W, H, FPS, DUR = 1920, 1080, 60, 6.0

raw = os.path.join(Q, "reference.bgra")
if not os.path.exists(raw):
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", REF, "-f", "rawvideo", "-pix_fmt", "bgra", raw], check=True)

COLOR = ["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"]
def vf(fmt, flags=None):
    f = "scale=out_color_matrix=bt709:out_range=tv"
    if flags:
        f += f":flags={flags}"
    return ["-vf", f + f",format={fmt}"]

CONFIGS = {
    # Current app settings (recording).
    "current_amf":     vf("nv12") + ["-c:v", "h264_amf", "-usage", "lowlatency", "-quality", "speed", "-rc", "cqp", "-qp_i", "22", "-qp_p", "22", "-qp_b", "22", "-g", "60", "-bf", "0"],
    "current_x264":    vf("yuv420p") + ["-c:v", "libx264", "-preset", "superfast", "-crf", "21", "-tune", "zerolatency", "-g", "60", "-bf", "0"],
    "current_mf":      vf("nv12") + ["-c:v", "h264_mf", "-b:v", "12441600", "-rate_control", "u_vbr", "-scenario", "display_remoting", "-g", "60", "-bf", "0"],
    # Candidates.
    "amf_q_qp18":      vf("nv12") + ["-c:v", "h264_amf", "-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", "18", "-qp_p", "18", "-qp_b", "18", "-g", "60", "-bf", "0"],
    "amf_hq_qp16":     vf("nv12") + ["-c:v", "h264_amf", "-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", "16", "-qp_p", "16", "-qp_b", "16", "-g", "60", "-bf", "0"],
    "amf_q_qp16_lz":   vf("nv12", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "h264_amf", "-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", "16", "-qp_p", "16", "-qp_b", "16", "-g", "60", "-bf", "0"],
    "amf_q_qp14_lz":   vf("nv12", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "h264_amf", "-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", "14", "-qp_p", "14", "-qp_b", "14", "-g", "60", "-bf", "0"],
    "amf_speed_qp16":  vf("nv12") + ["-c:v", "h264_amf", "-usage", "lowlatency", "-quality", "speed", "-rc", "cqp", "-qp_i", "16", "-qp_p", "16", "-qp_b", "16", "-g", "60", "-bf", "0"],
    "x264_vf_crf16":   vf("yuv420p", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-tune", "zerolatency", "-g", "60", "-bf", "0"],
    "x264_sf_crf14":   vf("yuv420p", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "libx264", "-preset", "superfast", "-crf", "14", "-tune", "zerolatency", "-g", "60", "-bf", "0"],
    "amf_hq3_qp18":    vf("nv12", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "h264_amf", "-usage", "transcoding", "-quality", "high_quality", "-rc", "cqp", "-qp_i", "18", "-qp_p", "18", "-qp_b", "18", "-g", "60", "-bf", "0"],
    "amf_q_qp19_lz":   vf("nv12", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "h264_amf", "-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", "19", "-qp_p", "19", "-qp_b", "19", "-g", "60", "-bf", "0"],
    "x264_med_crf18":  vf("yuv420p", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "libx264", "-preset", "medium", "-crf", "18", "-g", "60", "-bf", "0"],
    "x264_slow_crf15": vf("yuv420p", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "libx264", "-preset", "slow", "-crf", "15", "-g", "60", "-bf", "0"],
    "x264_vf_crf16_nozl": vf("yuv420p", "lanczos+accurate_rnd+full_chroma_int") + ["-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-g", "60", "-bf", "0"],
}

def metrics(path):
    lav = (f"[0:v]setpts=N/{FPS}/TB,scale=in_color_matrix=bt709:in_range=tv:flags=bicubic+accurate_rnd+full_chroma_int,format=gbrp[a];"
           f"[1:v]setpts=N/{FPS}/TB,format=gbrp[b];[a]split[a1][a2];[b]split[b1][b2];[a1][b1]ssim;[a2][b2]psnr")
    r = subprocess.run(["ffmpeg", "-hide_banner", "-i", path, "-i", REF, "-lavfi", lav, "-f", "null", "-"],
                       capture_output=True, text=True)
    ssim = float(re.search(r"SSIM .*All:([0-9.]+)", r.stderr).group(1))
    psnr = re.search(r"PSNR .*average:([0-9.inf]+)", r.stderr).group(1)
    ly = (f"[0:v]setpts=N/{FPS}/TB,scale=in_range=tv:out_range=tv,format=gray[a];[1:v]setpts=N/{FPS}/TB,scale=out_color_matrix=bt709:out_range=tv,format=gray[b];[a][b]ssim")
    r2 = subprocess.run(["ffmpeg", "-hide_banner", "-i", path, "-i", REF, "-lavfi", ly, "-f", "null", "-"], capture_output=True, text=True)
    ssim_y = float(re.search(r"SSIM .*All:([0-9.]+)", r2.stderr).group(1))
    return ssim, psnr, ssim_y

only = sys.argv[2:] or list(CONFIGS)
results = {}
for name in only:
    args = CONFIGS[name]
    out = os.path.join(Q, f"enc_{name}.mp4")
    cmd = ["ffmpeg", "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "bgra", "-video_size", f"{W}x{H}",
           "-framerate", str(FPS), "-i", "pipe:0"] + args + COLOR + [out]
    t0 = time.time()
    with open(raw, "rb") as f:
        p = subprocess.run(cmd, stdin=f, capture_output=True, text=True)
    el = time.time() - t0
    if p.returncode != 0:
        print(f"{name:16s} FAILED {p.stderr.strip()[-200:]}")
        continue
    size = os.path.getsize(out)
    ssim, psnr, ssim_y = metrics(out)
    results[name] = dict(ssim=ssim, psnr=psnr, ssim_y=ssim_y, mbps=size * 8 / DUR / 1e6, fps=FPS * DUR / el)
    print(f"{name:16s} SSIM(RGB) {ssim:.5f}  Y-SSIM {ssim_y:.5f}  PSNR {psnr:>6} dB  {size*8/DUR/1e6:6.2f} Mbit/s  {FPS*DUR/el:6.1f} fps")
json.dump(results, open(os.path.join(Q, "bench.json"), "a"))
