export function personalPageAiBootstrap(channel: string) {
  return `(() => {
    const channel=${JSON.stringify(channel)}, pending=new Map(), actions=new Map();
    window.addEventListener('message',event=>{const m=event.data;if(event.source!==parent||!m||m.channel!==channel||m.type!=='cilai-personal-ai-result')return;const p=pending.get(m.id);if(!p)return;pending.delete(m.id);clearTimeout(p.timer);m.error?p.reject(new Error(m.error)):p.resolve(m.result);});
    const request=(method,params={},requestId=crypto.randomUUID())=>new Promise((resolve,reject)=>{
      if(pending.size>=8){reject(new Error('页面请求过多'));return;}
      if(pending.has(requestId)){reject(new Error('请求正在处理'));return;}
      const timer=setTimeout(()=>{pending.delete(requestId);reject(new Error('请求结果未确认，请核对原请求，勿重复执行'));},60000);
      pending.set(requestId,{resolve,reject,timer});parent.postMessage({type:'cilai-personal-ai-request',channel,id:requestId,method,params},'*');
    });
    const ai=Object.freeze({request,
      session:Object.freeze({read:()=>request('session.read')}),
      context:Object.freeze({publish:context=>request('context.publish',context),revoke:contextId=>request('context.revoke',{contextId})}),
      uiAction:Object.freeze({register:async handlers=>{
        if(!handlers||typeof handlers!=='object'||Array.isArray(handlers)||Object.values(handlers).some(fn=>typeof fn!=='function'))throw new Error('页面动作无效');
        const entries=Object.entries(handlers), result=await request('uiAction.register',{actionIds:entries.map(([id])=>id)});
        actions.clear();for(const [id,fn] of entries)actions.set(id,fn);return result;
      }}),
      model:Object.freeze({session:Object.freeze({open:()=>request('model.session.open'),read:modelSessionId=>request('model.session.read',{modelSessionId})}),run:Object.freeze({create:params=>request('model.run.create',params),get:runId=>request('model.run.get',{runId}),cancel:runId=>request('model.run.cancel',{runId})})}),
      conversation:Object.freeze({handoff:runId=>request('conversation.handoff',{runId})}),
      audit:Object.freeze({list:(limit=50,beforeSequence)=>request('audit.list',{limit,...(beforeSequence===undefined?{}:{beforeSequence})})})
    });
    Object.defineProperty(window,'__cilaiPersonalAiExecute',{value:async request=>{
      const action=actions.get(request.actionId);if(!action)throw new Error('页面动作未登记');
      const result=await action(request.arguments);return result===undefined?null:JSON.parse(JSON.stringify(result));
    },writable:false,configurable:false});
    Object.defineProperty(window,'__cilaiPersonalAi',{value:ai,writable:false,configurable:false});
  })();`;
}

export function personalWebApplicationDocument(html: string, channel: string) {
  return `<script>${personalPageAiBootstrap(channel)}</script><script>Object.defineProperty(window,'cilaiPersonal',{value:Object.freeze({ai:window.__cilaiPersonalAi}),writable:false,configurable:false});</script>${html}`;
}
