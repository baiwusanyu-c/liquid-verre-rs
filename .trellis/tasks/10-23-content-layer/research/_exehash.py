import hashlib, os
R = r"D:\github-project\liquid-verre-rs"
research = os.path.join(R, ".trellis", "tasks", "10-23-content-layer", "research")
def sha(p):
    with open(p, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()[:16]
a = os.path.join(research, "vc_interleaved.exe")
b = os.path.join(research, "vc_batch_probe.exe")
c = os.path.join(R, r"target\release\liquid-verre-playground.exe")
for name, p in (("interleaved", a), ("batch_probe", b), ("target", c)):
    print(name, sha(p), os.path.getsize(p))
print("interleaved==batch_probe?", sha(a) == sha(b))
