const {spawn}=require('node:child_process');
const fs=require('node:fs');
const {chromium}=require(process.argv[2]||'playwright');
(async()=>{
  fs.mkdirSync('output/carousel-qa',{recursive:true});
  const server=spawn(process.execPath,['node_modules/vite/bin/vite.js','--config','scripts/display-controls-qa/vite.config.mjs'],{windowsHide:true,stdio:'pipe'});
  let log='',browser;server.stdout.on('data',b=>log+=b);server.stderr.on('data',b=>log+=b);
  try {
    for(let i=0;;i++){
      if(server.exitCode!==null)throw Error(log);
      try{if((await fetch('http://127.0.0.1:1427/scripts/display-controls-qa/index.html')).ok)break;}catch{}
      if(i===40)throw Error('QA server timeout');await new Promise(r=>setTimeout(r,500));
    }
    browser=await chromium.launch({channel:'msedge',headless:true});let count=0;
    for(const dpr of [1,1.75])for(const theme of ['light','dark']){
      const context=await browser.newContext({viewport:{width:2200,height:1000},deviceScaleFactor:dpr,colorScheme:theme});const page=await context.newPage();
      await page.goto('http://127.0.0.1:1427/scripts/display-controls-qa/index.html');await page.waitForFunction(()=>window.qa);
      for(const language of ['en-US','zh-CN','zh-Hant'])for(const overlayLayout of ['grouped','stacks'])for(const showProviderLogo of [true,false]){
        for(const provider of ['codex','claude','dsh','antigravity']){
          const sizes=[];
          for(const quotaCarouselEnabled of [false,true]){
            await page.evaluate(s=>window.qa.render(s,null,false,'overlay','carousel'),{provider,language,overlayLayout,showProviderLogo,quotaCarouselEnabled,scalePercent:175});
            const m=await page.evaluate(()=>{
              const capsule=document.querySelector('.overlay-shell > .overlay-capsule'),r=capsule.getBoundingClientRect();
              const stable=capsule.cloneNode(true);stable.querySelectorAll('.overlay-countdown').forEach(el=>el.textContent='COUNTDOWN');
              return {width:r.width,height:r.height,markup:stable.innerHTML,label:!!capsule.querySelector('.overlay-source-label'),
                clipped:[...capsule.querySelectorAll('span,button')].some(el=>{const b=el.getBoundingClientRect();return b.left<r.left-.2||b.right>r.right+.2||b.bottom>r.bottom+.2;})};
            });
            if(m.clipped||m.label)throw Error(JSON.stringify({dpr,theme,language,overlayLayout,showProviderLogo,provider,quotaCarouselEnabled,...m}));sizes.push(m);count++;
          }
          if(Math.abs(sizes[0].width-sizes[1].width)>1||Math.abs(sizes[0].height-sizes[1].height)>1||sizes[0].markup!==sizes[1].markup)throw Error('Carousel toggle changes capsule: '+JSON.stringify({provider,sizes}));
        }
        if(dpr===1&&theme==='dark'&&language==='zh-CN'&&overlayLayout==='grouped'&&!showProviderLogo)await page.screenshot({path:'output/carousel-qa/no-logo-zh-dark.png'});
      }
      for(const [width,height] of [[1320,800],[760,500]]){
        await page.setViewportSize({width,height});await page.evaluate(()=>window.qa.render({language:'zh-CN',quotaCarouselEnabled:true},null,false,'settings'));
        const slider=page.getByRole('slider',{name:'轮播间隔'});if(await slider.getAttribute('min')!=='3'||await slider.getAttribute('max')!=='30'||await slider.getAttribute('step')!=='1')throw Error('Slider contract changed');
        await slider.scrollIntoViewIfNeeded();if(!await slider.isVisible())throw Error('Slider unreachable');
        if(width===1320&&dpr===1&&theme==='dark')await page.screenshot({path:'output/carousel-qa/settings-zh-dark.png'});count++;
      }
      await context.close();
    }
    fs.writeFileSync('output/carousel-qa/results.json',JSON.stringify({passed:count}));console.log(`Carousel QA: ${count} combinations passed`);
  } finally {await browser?.close();server.kill();}
})().catch(e=>{console.error(e);process.exitCode=1;});
