"""Measure actual rendered text sizes, contrast and focus rings in both themes."""

import os
from playwright.sync_api import sync_playwright

AUDIT = r"""() => {
  const rgb = value => (value.match(/[\d.]+/g)||[]).slice(0,3).map(Number);
  const luminance = value => rgb(value).map(c => c/255).map(c => c<=.04045?c/12.92:((c+.055)/1.055)**2.4).reduce((a,c,i)=>a+c*[.2126,.7152,.0722][i],0);
  const ratio = (a,b) => { const x=luminance(a),y=luminance(b);return (Math.max(x,y)+.05)/(Math.min(x,y)+.05) };
  function background(element) {
    while(element){const color=getComputedStyle(element).backgroundColor;if(color!=='rgba(0, 0, 0, 0)' && color!=='transparent')return color;element=element.parentElement}
    return 'rgb(17,17,19)';
  }
  let errors=[], min=Infinity, lowest=Infinity, count=0;
  const walker=document.createTreeWalker(document.body,NodeFilter.SHOW_TEXT);
  while(walker.nextNode()) {
    const node=walker.currentNode,element=node.parentElement;
    if(!node.textContent.trim()||!element.getClientRects().length||element.closest('script,style,svg,[aria-hidden=true],textarea'))continue;
    const style=getComputedStyle(element),size=parseFloat(style.fontSize),contrast=ratio(style.color,background(element));
    const large=size>=24||(size>=18.66&&Number(style.fontWeight)>=700),required=large?3:4.5;
    if(size<12||contrast<required-.01)errors.push({text:node.textContent.trim().slice(0,70),size,contrast:+contrast.toFixed(2),foreground:style.color,background:background(element)});
    count++;min=Math.min(size,min);lowest=Math.min(lowest,contrast);
  }
  return {count,min,lowest:+lowest.toFixed(2),errors};
}"""
with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page()
    page.goto(os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765"))
    page.wait_for_function("window.engineHealth?.manim")
    page.evaluate("closeTour(false); setSource(EXAMPLES[0].source)")
    page.wait_for_function('!document.querySelector("#render-button").disabled')
    for theme in ("dark", "light"):
        page.evaluate("theme => applyTheme(theme)", theme)
        for width, height in ((1920, 1080), (1366, 768)):
            page.set_viewport_size({"width": width, "height": height})
            result = page.evaluate(AUDIT)
            assert not result["errors"], result
            for selector in (
                "#render-button",
                "#paste-button",
                "#import-button",
                "#theme-button",
                "#split-divider",
            ):
                page.locator(selector).focus()
                focus = page.locator(selector).evaluate(
                    "(e)=>{const s=getComputedStyle(e);return {style:s.outlineStyle,width:s.outlineWidth}}"
                )
                assert (
                    focus["style"] != "none"
                    and float(focus["width"].replace("px", "")) >= 2
                ), (selector, focus)
            print(
                f"PASS {theme} {width}x{height}: {result['count']} text runs, min {result['min']}px, lowest contrast {result['lowest']}:1; visible focus",
                flush=True,
            )
    browser.close()
