function utf8Prefix(text,limit){
  let bytes=new TextEncoder().encode(text);if(bytes.length<=limit)return text;
  bytes=bytes.slice(0,Math.max(0,limit));
  for(let i=0;i<4;i++){try{return new TextDecoder('utf-8',{fatal:true}).decode(bytes);}catch{bytes=bytes.slice(0,-1);}}
  return '';
}
export function sanitizeBoxText(bytes,cap=8192,secrets=[]){
  const raw=new TextDecoder('utf-8').decode(bytes),encoder=new TextEncoder();
  let text=raw.replace(/\r\n?/g,'\n').replace(/[\u2028\u2029]/g,'\n')
    .replace(/\x1b\][\s\S]*?(?:\x07|\x1b\\|$)/g,'')
    .replace(/\x1b[P^_][\s\S]*?(?:\x1b\\|$)/g,'')
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g,'')
    .replace(/\x1b[ -/]*[@-~]/g,'')
    .replace(/[\x00-\x08\x0b-\x1f\x7f-\x9f]/g,'')
    .replace(/\p{Cf}/gu,'') // MUTANT unicode-format
    .replace(/\b(?:gh[pousr]_[A-Za-z0-9]{20,255}|github_pat_[A-Za-z0-9_]{20,255}|v1\.[0-9]+\.[0-9]+\.[0-9]+\.[A-Za-z0-9_-]{43})\b/g,'[redacted]'); // MUTANT token-shape
  for(const secret of secrets)if(secret)text=text.split(secret).join('[redacted]');
  text=text.replaceAll('::',': :').replaceAll('##[','# #['); // MUTANT command-delimiters
  const lines=text.split('\n');if(lines.at(-1)==='')lines.pop();
  let retained=[],size=0,shown=0;
  for(const line of lines.slice(-200)){const safe=utf8Prefix(line,1018);const rendered='[box] '+safe+'\n'; // MUTANT all-prefix
    const length=encoder.encode(rendered).length;retained.push({rendered,length,shown:encoder.encode(safe).length+1});size+=length;shown+=encoder.encode(safe).length+1;
    while(size>cap||retained.length>200){const old=retained.shift();size-=old.length;shown-=old.shown;}
  }
  return {text:retained.map(x=>x.rendered).join(''),bytesDropped:Math.max(0,bytes.byteLength-shown)};
}
export function appendBoxText(record,bytes,secrets=[]){
  const next=sanitizeBoxText(bytes,8192,secrets);
  record.bytesDropped=(record.bytesDropped||0)+next.bytesDropped;
  const lines=((record.log||'')+next.text).split('\n');if(lines.at(-1)==='')lines.pop();
  let size=new TextEncoder().encode(lines.join('\n')+(lines.length?'\n':'')).length;
  while(size>8192||lines.length>200){const old=lines.shift();const length=new TextEncoder().encode(old+'\n').length;size-=length;record.bytesDropped+=Math.max(0,length-6);}
  record.log=lines.length?lines.join('\n')+'\n':'';
}
export function sanitizeLog(bytes,cap=8192,secrets=[]){return sanitizeBoxText(bytes,cap,secrets).text;}
