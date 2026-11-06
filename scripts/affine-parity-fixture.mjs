// Synthetic mature-feedback trajectory. No local account or usage history is read.
import {readFileSync, writeFileSync, mkdirSync} from 'node:fs';
const path = new URL('../src-tauri/tests/fixtures/affine-parity.csv',import.meta.url);
const clamp=(x,a,b)=>Math.max(a,Math.min(b,x));
let weight=0,sumX=0,sumY=0,sumXX=0,sumXY=0;
const lines=['base,elapsed,observed,confidence,capped,at,expected'];
for(let i=0;i<96;i++){
  const base=i%11===0?0:((i*37)%360)/10;
  const elapsed=300+(i%4)*30, observed=i%5===0?0:((i*7)%19)/4, confidence=i%7===0?0:1;
  const raw=base*elapsed/3600, capped=i%9===0?raw*.6:raw, at=10000+i*400;
  const x=base/12, determinant=(sumXX+10)*(weight+10)-sumX*sumX;
  const slope=clamp(((sumXY+10)*(weight+10)-sumY*sumX)/determinant,.5,1.5);
  const intercept=clamp(((sumXX+10)*sumY-sumX*(sumXY+10))/determinant,-1.5,1.5);
  const prediction=weight===0?base:clamp(x+weight/(weight+8)*(slope*x+intercept-x),.75*x,1.5*x)*12;
  lines.push([base,elapsed,observed,confidence,capped,at,prediction].join(','));
  weight*=.99;sumX*=.99;sumY*=.99;sumXX*=.99;sumXY*=.99;
  if(confidence>0 && raw<=capped+1e-8){
    const y=observed*300/elapsed;weight++;sumX+=x;sumY+=y;sumXX+=x*x;sumXY+=x*y;
  }
}
const output=lines.join('\n')+'\n';
if(process.argv.includes('--check')){
  if(readFileSync(path,'utf8').replace(/\r\n/g,'\n')!==output)throw new Error('Synthetic calibration fixture differs');
}else{mkdirSync(new URL('.',path),{recursive:true});writeFileSync(path,output);}
console.log('96 synthetic causal origins; fixture '+(process.argv.includes('--check')?'verified':'written'));
