let r=document.body,s,x,d=new TextDecoder,g=(p,l)=>d.decode(new Uint8Array(x.m.buffer,p,l));
WebAssembly.instantiateStreaming(fetch("counter.wasm"),{d:{h:(p,l)=>{r.innerHTML=g(p,l);s=r.querySelector("u")},t:(p,l)=>s.textContent=g(p,l)}}).then(o=>{x=o.instance.exports;x.s();r.onclick=e=>{e=e.target.closest("[data-e]");e&&x.e(e.dataset.e)}})
