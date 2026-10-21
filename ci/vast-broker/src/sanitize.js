export function parseResult(bytes,nonce){
  let text;try{text=new TextDecoder('utf-8',{fatal:true}).decode(bytes);}catch{return null;}
  if(!text.endsWith('\n'))return null;
  const lines=text.split('\n').filter(x=>x.length!==0);
  const marker=lines.at(-1)?.match(/^KF_RESULT ([0-9a-f]{32}) (PASS|FAIL) ([0-9a-f]{64})$/);
  if(!marker||marker[1]!==nonce||lines.filter(x=>x.startsWith('KF_RESULT ')).length!==1)return null;
  return marker[2];
}
export function sanitizeLog(bytes,cap=8192,secrets=[]){
  let text=new TextDecoder('utf-8').decode(bytes)
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g,'')
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g,'')
    .replace(/[\x00-\x08\x0b-\x1f\x7f-\x9f]/g,'')
    .replace(/\p{Cf}/gu,'') // MUTANT unicode-format
    .replace(/\b(?:gh[pousr]_[A-Za-z0-9]{20,255}|github_pat_[A-Za-z0-9_]{20,255})\b/g,'[redacted]') // MUTANT token-shape
    .replace(/^(::|##\[)/gm,'[box] $1');
  for(const secret of secrets)if(secret&&secret.length>=4)text=text.split(secret).join('[redacted]');
  const encoder=new TextEncoder();const encoded=encoder.encode(text);
  if(encoded.length<=cap)return text;
  const room=Math.max(0,Math.floor((cap-100)/2));
  // Fatal decoding trims partial code points at each end of the excerpt.
  let head=encoded.slice(0,room),tail=encoded.slice(encoded.length-room);
  for(let n=0;n<4;n++){try{new TextDecoder('utf-8',{fatal:true}).decode(head);break;}catch{head=head.slice(0,-1);}}
  for(let n=0;n<4;n++){try{new TextDecoder('utf-8',{fatal:true}).decode(tail);break;}catch{tail=tail.slice(1);}}
  const excerpt=new TextDecoder().decode(head)+`\n[box: ${encoded.length-head.length-tail.length} bytes dropped]\n`+new TextDecoder().decode(tail);
  return excerpt.replace(/^(::|##\[)/gm,'[box] $1'); // MUTANT excerpt-prefix
}
