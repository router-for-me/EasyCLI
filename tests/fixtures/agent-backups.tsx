import React from 'react';
import { createRoot } from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { AgentsPage } from '../../src/pages/AgentsPage';
import { I18nProvider } from '../../src/i18n';
import '../../src/styles/index.css';
const params = new URLSearchParams(location.search);
if(params.has('reset-selections'))localStorage.removeItem('cpa-gui.agent-model-selections.v1');
localStorage.setItem('easy-cli-proxy-api.locale',params.get('locale') || 'zh-CN');
document.documentElement.dataset.theme = params.get('theme') || 'light';
localStorage.setItem('cpa-gui.agent-selected-client.v1', params.get('client') || 'codex');
const ids = ['claude-code','claude-desktop','codex','opencode','openclaw','hermes','deepseek-harness','antigravity-cli','workbuddy','zcode','kimi-code','grok-build','omp','pi'];
let count=0; let backupCount=0; const backups:any[]=[]; let currentModel=params.has('fresh')?null:'gpt-one';
let nativeOauth=params.has('native-oauth'); let nativeSwitchCount=0; let codexClosed=false; let closeCount=0;
let currentOauth=false; const currentMappings:Record<string,any>={};
const claudeAliasLayoutModels = [
 {name:'gpt-6.1-sol-high-fast',alias:'gpt-6.1-sol',isAlias:true},
 {name:'gpt-6.1-sol',isAlias:true},
 {name:'provider/very-long-upstream-model-name-with-reasoning-high',alias:'provider/very-long-original-model-name-with-context-window',isAlias:true},
];
if(params.has('claude-alias-layout'))currentMappings['claude-code']={opus:claudeAliasLayoutModels[0].name,sonnet:claudeAliasLayoutModels[0].name,fable:claudeAliasLayoutModels[1].name,haiku:claudeAliasLayoutModels[2].name,startupModel:'opus',subagentModel:''};
if(params.has('legacy-desktop'))currentMappings['claude-desktop']={opus:'gpt-one',sonnet:'gpt-two',haiku:'gpt-one',opus1m:true};
if(params.has('legacy-desktop-direct'))currentMappings['claude-desktop']={opus:'',sonnet:'claude-sonnet-custom-7',haiku:'',desktopModels:[{model:'',alias:'claude-sonnet-custom-7',context1m:false}]};
if(params.has('saved-desktop-alias'))currentMappings['claude-desktop']={opus:'',sonnet:'gpt-one',haiku:'',desktopModels:[{model:'gpt-one',alias:'claude-opus-5',context1m:false}]};
let harnessProvider:Record<string,unknown>={};
const harnessConfigurations:Record<string,Record<string,unknown>>={};
let harnessRevision=1;
const harnessModels=[
 {id:'gpt-one',defaults:{input:['text','image'],contextWindow:128000}},
 {id:'gpt-two',defaults:{input:['text']}},
 {id:'unknown-model',defaults:{}},
];
const harnessSnapshot=()=>({revision:String(harnessRevision),provider:harnessProvider,baseUrl:'http://127.0.0.1:8317/v1',defaultModel:currentModel,configured:!!currentModel,models:harnessModels.map(m=>({...m,configuration:harnessConfigurations[m.id]??{}}))});
let harnessStatus={running:params.has('running'),pid:params.has('running')?100:null as number|null,mode:params.has('running')?params.get('harness-mode')||'web':null as string|null};
let appliedCount=0;
let detectionCount=0;
(window as any).fixtureOauthLoggedIn=!params.has('no-oauth-login');
const calls:any[]=[];(window as any).fixtureCalls=calls;
let embedded=params.has('embedded');
(window as any).fixtureSessionIds=Array.from({length:61},(_,index)=>`session-${index+1}`);
mockIPC(async (cmd,args:any) => {
 calls.push({cmd,args});
 if(cmd==='plugin:dialog|open') return (window as any).fixtureSelectedProgram ?? null;
 if((window as any).fixtureDeferredCommands?.includes(cmd)) {
   await new Promise<void>(resolve=>{
     ((window as any).fixturePendingCommands??=[]).push({cmd,resolve});
   });
 }
 if((window as any).fixtureFailedCommands?.includes(cmd))throw new Error('模拟后台刷新失败');
 if(cmd==='plugin:event|listen') return 1;
 if(cmd==='plugin:event|unlisten'||cmd==='set_app_locale') return null;
 if(cmd==='get_agent_config_statuses'||cmd==='refresh_agent_config_statuses') {
   if(params.has('fail-detection')&&++detectionCount===1)throw new Error('模拟客户端检测失败');
   return ids.map(id=>({id,name:id,supportedPlatform:true,installed:!params.has('not-installed')&&params.get('config-only')!==id,pluginInstalled:!params.has('no-plugin'),launchTargets:params.has('not-installed')||params.get('config-only')===id?[]:['claude-desktop','zcode','workbuddy'].includes(id)?[{id:'app',label:id,detail:'test desktop'}]:['codex','opencode'].includes(id)&&!params.has('cli-only')?[...(params.has('app-only')?[]:[{id:'cli',label:'CLI',detail:'test CLI'}]),{id:'app',label:'APP',detail:'test desktop'}]:[{id:'cli',label:'CLI',detail:'test CLI'}],version:'1.0',cliVersion:'1.0',appVersion:null,pluginVersion:'1.0',configExists:!params.has('not-installed')||params.get('config-only')===id,configValid:params.get('state')!=='invalid',connectionState:params.get('state') || (currentModel?'configured':'not-configured'),configured:!!currentModel,configurationSynchronized:!!currentModel,currentModel,oauthConfiguration:id==='codex'&&currentOauth,codexNativeOauth:id==='codex'&&nativeOauth,modificationEnabled:!!currentModel,modificationState:currentModel?'applied':'unconfigured',backupAvailable:false,appliedModel:currentModel,claudeCodeModelMappings:id==='claude-code'?currentMappings[id]??null:null,claudeDesktopModelMappings:id==='claude-desktop'?currentMappings[id]??null:null,warnings:[],error:null}))
     .map(status=>status.id==='codex'&&(nativeOauth||codexClosed)?{...status,configured:false,connectionState:'not-configured',configurationSynchronized:false,currentModel:null,appliedModel:null,modificationEnabled:false,modificationState:'unconfigured',oauthConfiguration:false}:status)
     .map(status=>{
       if(!params.has('mixed-clients'))return status;
       const installed=['claude-code','codex','opencode'].includes(status.id);
       return {...status,installed,pluginInstalled:status.id==='pi',configExists:installed||status.id==='claude-desktop',configured:installed&&!!currentModel,configurationSynchronized:installed&&!!currentModel,connectionState:installed&&currentModel?'configured':'not-configured',currentModel:installed?currentModel:null,appliedModel:installed?currentModel:null,modificationEnabled:installed&&!!currentModel,modificationState:installed&&currentModel?'applied':'unconfigured',launchTargets:installed?status.launchTargets:[],version:installed?status.version:null,cliVersion:installed?status.cliVersion:null,pluginVersion:status.id==='pi'?'1.0':null};
     })
     .map(status=>({...status,...((window as any).fixtureClientStatusesOverride?.[status.id]??{})}));
 }
 if(cmd==='close_codex_config_modification') {
   closeCount++;
   if(params.has('fail-close')&&closeCount===1)throw new Error('模拟关闭失败，已回滚');
   codexClosed=true;nativeOauth=false;
   return {outcome:'updated',enabled:false,changedFiles:[],conflictFiles:[]};
 }
 if(cmd==='restore_codex_official_config') {
   nativeSwitchCount++;
   if(params.has('fail-native-oauth')&&nativeSwitchCount===1)throw new Error('模拟切换失败，已回滚');
   nativeOauth=true; return {outcome:'updated'};
 }
 if(cmd==='get_agent_models') {
   if((window as any).fixtureModelsOverride !== undefined)return (window as any).fixtureModelsOverride;
   if(params.has('claude-alias-layout'))return claudeAliasLayoutModels;
   if(params.has('no-models'))return [];
   if(params.has('no-core'))throw new Error('CPA core is offline');
   if(params.has('defer-models'))await new Promise<void>(resolve=>{(window as any).fixtureFinishModels=resolve;});
   if(args.client==='deepseek-harness')return harnessModels.map(m=>({name:m.id,inputModalities:m.defaults.input}));
   if(params.has('claude-models')||params.has('claude-alias-models')||params.has('saved-desktop-alias'))return [{name:'gpt-one'},{name:'claude-opus-5',isAlias:!params.has('claude-models')},{name:'claude-sonnet-5',isAlias:!params.has('claude-models')}];
   return [{name:'gpt-one'},{name:'gpt-two'}];
 }
 if(cmd==='list_codex_sessions') {
   const {offset,limit}=args.request;
   const sessionIds:string[]=[...(window as any).fixtureSessionIds];
   if((window as any).fixtureDeferSessionLoad) {
     (window as any).fixtureDeferSessionLoad=false;
     await new Promise<void>(resolve=>{(window as any).fixtureFinishSessionLoad=resolve;});
   }
   if((window as any).fixtureFailSessionLoad)throw new Error('模拟会话读取失败');
   return {codexHome:'C:/test/.codex',databasePaths:['C:/test/.codex/state.sqlite'],totalCount:sessionIds.length,offset,limit,hasMore:offset+limit<sessionIds.length,warnings:[],sessions:sessionIds.slice(offset,offset+limit).map(id=>({id,title:id,cwd:'C:/test/project',modelProvider:'test',archived:false,updatedAtMs:null,databasePath:'C:/test/.codex/state.sqlite'}))};
 }
 if(cmd==='get_deepseek_harness_process_status') return harnessStatus;
 if(cmd==='restart_agent_app'||cmd==='restart_deepseek_harness_process') {
   if(params.has('defer-restart'))await new Promise<void>(resolve=>{(window as any).fixtureFinishRestart=resolve;});
   if(params.has('fail-restart')) { if(cmd==='restart_deepseek_harness_process')harnessStatus={running:false,pid:null,mode:null}; throw new Error('模拟重启失败'); }
   if(cmd==='restart_deepseek_harness_process')return harnessStatus={running:true,pid:101,mode:'web'};
   return null;
 }
 if(cmd==='stop_deepseek_harness_process')return harnessStatus={running:false,pid:null,mode:null};
 if(cmd==='launch_agent')return null;
 if(cmd==='clear_codex_config') { currentModel=null; return []; }
 if(cmd==='set_agent_config_enabled' && !args.enabled) {
   if(params.has('fail-clear'))throw new Error('模拟清除失败，已回滚');
   if(params.has('defer-clear'))await new Promise<void>(resolve=>{(window as any).fixtureFinishClear=resolve;});
   currentModel=null;delete currentMappings[args.client];
   return {outcome:'updated',enabled:false,model:null,changedFiles:[],conflictFiles:[]};
 }
 if(cmd==='uninstall_pi_provider')return null;
 if(['update_agent_config','repair_pi_provider','install_pi_provider','update_pi_provider'].includes(cmd)) {
   count++;if(params.has('fail-apply')&&count===1)throw new Error('模拟配置写入失败');
   currentModel=args.model;currentOauth=!!args.oauthConfiguration;nativeOauth=false;codexClosed=false;
   currentMappings[args.client]=args.claudeCodeModelMappings??args.claudeDesktopModelMappings;
   return {outcome:count>1?'unchanged':'updated',model:currentModel,enabled:true,changedFiles:[],conflictFiles:[]};
 }
 if(cmd==='create_agent_config_backup') {
   const id=String(++backupCount);
   const files=[{path:'C:/test/.codex/config.toml',exists:true,size:120},{path:'C:/test/.codex/models.json',exists:true,size:60},{path:'C:/test/.codex/auth.json',exists:false,size:null}];
   const backup={id,createdAt:'2026-09-10T12:00:00Z',fileCount:3,location:'C:/CPA/backups/agents/'+args.client+'/'+id+'.json',files,restorable:params.get('state')!=='invalid',error:params.get('state')==='invalid'?'备份含有无法解析的配置':null,savedModel:currentModel};
   backups.unshift(backup);return backup;
 }
 if(cmd==='list_agent_config_backups') return {versions:backups};
 if(cmd==='preview_agent_config_backup') return {revision:'rev1',files:backups.find(b=>b.id===args.id).files,differences:[{file:'C:/test/.codex/config.toml',field:'file',before:'present',after:'replace'}]};
 if(cmd==='restore_agent_config_backup') {if(params.has('conflict'))throw new Error('预览后配置或备份发生变化，请重新预览');currentModel=backups.find(b=>b.id===args.id).savedModel;return {outcome:'updated'};}
 if(cmd==='delete_agent_config_backup') {backups.splice(backups.findIndex(b=>b.id===args.id),1);return null;}
 if(cmd==='preview_agent_config_template') return {revision:'template1',files:['C:/test/.codex/config.toml','C:/test/.codex/auth.json','C:/test/.codex/models.json']};
 if(cmd==='apply_agent_config_template') {currentModel=args.model;return {outcome:'updated'};}
 if(cmd==='check_pi_provider_update') return {installedVersion:'1.0',latestVersion:params.has('update')?'1.1':'1.0',updateAvailable:params.has('update')};
 if(cmd==='get_deepseek_harness_model_catalog_editor')return harnessSnapshot();
 if(cmd==='save_deepseek_harness_model_catalog_editor') {
   if((window as any).fixtureFailHarnessSave)throw new Error('模拟模型配置保存失败');
   if((window as any).fixtureStaleHarnessSave)throw new Error('DSH_MODEL_CATALOG_CHANGED');
   if((window as any).fixtureDeferHarnessSave)await new Promise<void>(resolve=>{(window as any).fixtureFinishHarnessSave=resolve;});
   harnessProvider=args.request.provider;
   for(const m of args.request.models)harnessConfigurations[m.id]=m.configuration;
   harnessRevision++;return harnessSnapshot();
 }
 if(cmd==='get_codex_model_catalog_editor') return {models:[],hiddenModels:[],customizations:{}};
 if(cmd==='check_codex_oauth_login') {
   if(!(window as any).fixtureOauthLoggedIn)throw new Error('CODEX_OAUTH_LOGIN_REQUIRED');
   return null;
 }
 throw new Error('Unhandled fixture command: '+cmd);
});
let root=createRoot(document.getElementById('root')!);
const render=()=>{
 const agents=<AgentsPage embedded={embedded} onConfigurationApplied={()=>{document.documentElement.dataset.fixtureApplied=String(++appliedCount);}}/>;
 root.render(<I18nProvider>{params.has('shell')?(
  <div className="app-shell">
   <aside className="sidebar" aria-hidden="true" />
   <div className="workspace"><main className="content">{agents}</main></div>
  </div>
 ):agents}</I18nProvider>);
};
(window as any).fixtureRemount=(nextEmbedded=embedded)=>{embedded=nextEmbedded;root.unmount();root=createRoot(document.getElementById('root')!);render();};
render();
