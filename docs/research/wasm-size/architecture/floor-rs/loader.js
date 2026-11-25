let x,T=[],S=[],r=document.body,d=new TextDecoder;
const f=(p,l)=>{const m=new DataView(x.memory.buffer,p,l),u=new Uint8Array(x.memory.buffer,p,l);for(let i=0,a,n,e;i<l;){switch(m.getUint8(i++)){
case 1:a=m.getUint8(i++);n=m.getUint16(i,1);i+=2;(T[a]=document.createElement("template")).innerHTML=d.decode(u.subarray(i,i+=n));break;
case 2:a=m.getUint8(i++);n=m.getUint16(i,1);i+=2;e=T[a].content.cloneNode(!0);for(const k of e.querySelectorAll("[data-n]"))S[n+ +k.dataset.n]=k;r.append(e);break;
case 3:a=m.getUint16(i,1);n=m.getUint16(i+2,1);i+=4;S[a].textContent=d.decode(u.subarray(i,i+=n))}}};
WebAssembly.instantiateStreaming(fetch("floor.wasm"),{env:{f}}).then(o=>{x=o.instance.exports;x.start();r.onclick=e=>{e=e.target.closest("[data-n]");e&&x.ev(S.indexOf(e),0)}})
