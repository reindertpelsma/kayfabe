from pathlib import Path
import re,json
new=Path('/root/prov/ada590_gfx_profile_corrected.log').read_text()
native=Path('/root/prov/ada2build590_native_gfx.log').read_text()
assert all('GFX_S'+str(i)+'=PASS' in new for i in range(1,6)),'corrected graphics step failed'
def hashes(s):return dict(re.findall(r'^(VKC_HASH|VKR_HASH_[AZB]|EGLR_HASH_[AZB]|GLXR_HASH_[AZB])=([^\r\n]+)',s,re.M))
a,b=hashes(new),hashes(native);assert len(a)==10 and a==b,(a,b)
print('GFX590_NATIVE_PARITY_PASS',json.dumps(a,sort_keys=True))
