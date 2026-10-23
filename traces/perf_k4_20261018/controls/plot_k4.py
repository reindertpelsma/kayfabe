"""Small standalone scientific plots from typed public evidence, no vendor scores."""
import json
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
P=Path('/workspace/q4-wt/traces/perf_k4_20261018');c=json.loads((P/'cohort.json').read_text());p=json.loads((P/'proxy-results.json').read_text())
plt.rcParams.update({'font.size':10,'axes.spines.top':False,'axes.spines.right':False,'savefig.dpi':160})
colors={'kayfabe':'#00779a','VFIO':'#bd6527'}
fig,axs=plt.subplots(2,3,figsize=(12,6.4),layout='constrained')
metrics=[('util_gpu','Global GPU busy (%)',1),('clock_sm_mhz','SM clock (MHz)',1),('clock_mem_mhz','Memory clock (MHz)',1),('power_mw','Power (W)',.001),('pcie_rx_kb_s','PCIe RX (API KB/s)',1),('pcie_tx_kb_s','PCIe TX (API KB/s)',1)]
for ax,(key,title,scale) in zip(axs.flat,metrics):
 for i,w in enumerate(('catia-07','creo-04')):
  for arm,x in [('kayfabe',i-.13),('VFIO',i+.13)]:
   r=next(r for r in c['aggregate'] if r['workload']==w and r['arm']==arm);v=r['physical_metrics'][key];ys=[y*scale for y in v['boot_means']]
   ax.plot([x,x],[min(ys),max(ys)],color=colors[arm],lw=2);ax.scatter([x-.025,x+.025],ys,color=colors[arm],s=30,label=arm if i==0 else None,zorder=3)
 ax.set_xticks([0,1],['CATIA','Creo']);ax.set_title(title);ax.set_xlim(-.5,1.5);ax.set_ylim(bottom=0);ax.grid(axis='y',alpha=.2)
axs.flat[0].legend(loc='lower right');fig.suptitle('K4 measured physical telemetry — two fresh boots per arm',fontsize=14)
fig.supxlabel(f"Runs4642/4649 kayfabe714ec29d3;4648/4643 VFIO. Common boundary exclusion {c['common_boundary_margin_seconds']:.3f}s. Dots are boot means; rolling polls are correlated.",fontsize=8)
fig.savefig(P/'physical-telemetry.png');plt.close(fig)
fig,axs=plt.subplots(1,3,figsize=(12,4.5),layout='constrained')
for ax,(variant,vertices,title) in zip(axs,[('primary',3000,'Primary:3000 vertices/draw'),('primary',300000,'Primary:300000 vertices/draw'),('windowed',3000,'Visible window:3000 vertices/draw')]):
 for i,mode in enumerate(('offscreen','present')):
  for arm,x in [('kayfabe',i-.13),('VFIO',i+.13)]:
   r=next(r for r in p['groups'] if r['variant']==variant and r['vertices']==vertices and r['mode']==mode and r['arm']==arm);v=list(r['boot_means'].values())
   ax.plot([x,x],[r['phase_range'][0],r['phase_range'][1]],color=colors[arm],lw=2)
   ax.scatter([x]*len(v),v,color=colors[arm],label=arm if i==0 else None,s=34,zorder=3)
 ax.set_title(title);ax.set_xticks([0,1],['No Present','Present(0)']);ax.set_yscale('log');ax.set_ylim(30,10000);ax.grid(axis='y',which='both',alpha=.2);ax.set_xlim(-.5,1.5)
axs[0].set_ylabel('Completed loop iterations/s (log scale)');axs[0].legend(loc='lower left')
fig.suptitle('K4 generated D3D11 proxy — completed calls, not displayed fps',fontsize=14)
fig.supxlabel('Primary:n2 boots/arm,3 phases/mode/boot; unequal desktop coverage. Windowed:n1 boot/arm,1280x720 client,1920x1080 buffer. Lines show phase ranges.\nThis is not a replay of CATIA/Creo OpenGL; GPU timestamp spans are not active kernel time.',fontsize=8)
fig.savefig(P/'present-proxy.png');plt.close(fig)
print('Wrote physical-telemetry.png and present-proxy.png')
