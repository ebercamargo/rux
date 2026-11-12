use std::{fs, path::Path};

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn create(name: &str, fields: &[String]) -> Result<(), String> {
    let typed = fields.iter().map(|f| (f.clone(), "Text".to_string())).collect::<Vec<_>>();
    create_typed(name, &typed)
}

fn render_typed_ui(name: &str, typed: &[(String, String)]) -> String {
    let fields_js = typed.iter().map(|(field, _)| format!("\"{field}\"")).collect::<Vec<_>>().join(",");
    let types_js = typed.iter().map(|(field, ty)| format!("\"{field}\":\"{ty}\"")).collect::<Vec<_>>().join(",");
    TEMPLATE.replace("__RUX_TITLE__", name).replace("__RUX_FIELDS__", &fields_js).replace("__RUX_TYPES__", &types_js)
}

pub fn refresh_typed_ui(name: &str, typed: &[(String, String)]) -> Result<(), String> {
    if !valid_name(name) { return Err("Invalid CRUD name".into()); }
    if typed.is_empty() || typed.len() > 20 { return Err("CRUD must declare 1-20 fields".into()); }
    let mut seen = std::collections::HashSet::new();
    if typed.iter().any(|(field, _)| !valid_name(field) || field == "id" || !seen.insert(field)) {
        return Err("Invalid or duplicate CRUD field".into());
    }
    let directory = Path::new(name);
    if !directory.is_dir() { return Err(format!("CRUD directory does not exist: {name}")); }
    let html = render_typed_ui(name, typed);
    fs::write(directory.join("index.html"), html).map_err(|e| format!("cannot refresh UI: {e}"))?;
    let manifest = format!(r###"{{"name":"RUX {name}","short_name":"{name}","start_url":"/","display":"standalone","background_color":"#f5f6fa","theme_color":"#4d49bd"}}"###);
    fs::write(directory.join("manifest.webmanifest"), manifest)
        .map_err(|e| format!("cannot refresh manifest: {e}"))?;
    Ok(())
}

pub fn create_typed(name: &str, typed: &[(String, String)]) -> Result<(), String> {
    let fields = typed.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();
    if !valid_name(name) { return Err("CRUD name must contain letters, digits or underscore and start with a letter".into()); }
    if fields.is_empty() { return Err("Provide at least one field, e.g. rux crud clientes nome email telefone".into()); }
    if fields.len() > 20 || fields.iter().any(|f| !valid_name(f) || f == "id") {
        return Err("Use 1-20 valid field names; 'id' is reserved".into());
    }
    let mut unique = std::collections::HashSet::new();
    if fields.iter().any(|f| !unique.insert(f)) { return Err("Duplicate CRUD field".into()); }
    let directory = Path::new(name);
    if directory.exists() { return Err(format!("Directory already exists: {name}")); }
    let fields_js = fields.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(",");
    let html = render_typed_ui(name, typed);
    fs::create_dir(directory).map_err(|e| e.to_string())?;
    fs::write(directory.join("index.html"), html).map_err(|e| e.to_string())?;
    let types_js = typed.iter().map(|(field, ty)| format!("\"{field}\":\"{ty}\"")).collect::<Vec<_>>().join(",");
    let server = include_str!("crud_server.js").replace("__RUX_FIELDS__", &fields_js).replace("__RUX_TYPES__", &types_js);
    fs::write(directory.join("server.js"), server).map_err(|e| e.to_string())?;
    let manifest = format!(r###"{{"name":"RUX {name}","short_name":"{name}","start_url":"/","display":"standalone","background_color":"#f5f6fa","theme_color":"#4d49bd"}}"###);
    fs::write(directory.join("manifest.webmanifest"), manifest).map_err(|e| e.to_string())?;
    println!("CRUD created: {name}/index.html");
    println!("  cd {name}");
    println!("  node server.js");
    println!("Open http://localhost:3000 in your browser.");
    println!("Server data is saved in records.json. Bind is localhost only; native apps and authentication are not implemented yet.");
    Ok(())
}

const TEMPLATE: &str = r##"<!doctype html>
<html lang="pt-BR">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>RUX | __RUX_TITLE__</title><link rel="manifest" href="/manifest.webmanifest">
<style>
:root{font-family:system-ui,-apple-system,sans-serif;color-scheme:light;background:#f5f6fa;color:#202333}
*{box-sizing:border-box}body{margin:0;padding:24px}main{max-width:1050px;margin:auto}
h1{margin:0 0 8px}p{color:#515769}section{background:white;border:1px solid #e2e5ee;border-radius:14px;padding:22px;margin-top:20px}
form{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:14px;align-items:end}
label{display:grid;gap:6px;font-weight:600}input{width:100%;padding:12px;border:1px solid #bfc5d2;border-radius:8px;font:inherit}
button{padding:11px 15px;border:0;border-radius:8px;background:#4d49bd;color:white;font:inherit;cursor:pointer}
button.secondary{background:#596273}button.danger{background:#b5263d}
.actions{display:flex;gap:8px;flex-wrap:wrap}.toolbar{display:flex;justify-content:space-between;align-items:center;gap:12px;flex-wrap:wrap}
#search{max-width:320px}.pager{display:flex;align-items:center;gap:12px;justify-content:flex-end;margin-top:14px}button:disabled{opacity:.5;cursor:not-allowed}table{width:100%;border-collapse:collapse}th,td{text-align:left;padding:12px;border-bottom:1px solid #e5e7ee}
.table-scroll{overflow-x:auto}#status{min-height:24px}button:focus-visible,input:focus-visible{outline:3px solid #aaa5ff}
@media(max-width:640px){body{padding:12px}section{padding:14px}th,td{padding:9px}}
</style>
</head>
<body><main>
<header><h1>CRUD __RUX_TITLE__</h1><p>RUX • Cadastro multiplataforma no navegador</p></header>
<section><h2 id="form-title">Novo registro</h2><form id="record-form"></form><p id="status" role="status" aria-live="polite"></p></section>
<section><div class="toolbar"><h2>Registros</h2><input id="search" type="search" placeholder="Pesquisar..." aria-label="Pesquisar registros"></div>
<div class="table-scroll"><table><thead id="table-head"></thead><tbody id="table-body"></tbody></table></div><div class="pager"><button id="previous" type="button" class="secondary">Anterior</button><span id="page-label">Página 1</span><button id="next" type="button" class="secondary">Próxima</button></div></section>
</main>
<script>
"use strict";
const fields=[__RUX_FIELDS__];
const fieldTypes={__RUX_TYPES__};
function applyDigitMask(value,pattern){
 const digits=String(value).replace(/[^0-9]/g,"");
 let result="",position=0;
 for(const symbol of pattern){
  if(position>=digits.length)break;
  if(symbol==="9")result+=digits[position++];
  else result+=symbol;
 }
 return result;
}
function datePatternParts(pattern){
 const tokens=pattern.match(/yyyy|yy|MMMM|MMM|MM|dd|HH|hh|mm|ss|M|d|H|h|m|s|S{1,9}|a/g)||[];
 const numeric=tokens.every(t=>/^(yyyy|yy|MM|dd|HH|hh|mm|ss|M|d|H|h|m|s|S{1,9})$/.test(t));
 return {tokens,numeric};
}
function applyDateMask(value,pattern){
 const parts=datePatternParts(pattern);
 if(!parts.numeric)return value;
 const digits=String(value).replace(/\D/g,"");
 let index=0,result="",cursor=0;
 const re=/yyyy|yy|MM|dd|HH|hh|mm|ss|M|d|H|h|m|s|S{1,9}/g;
 for(const match of pattern.matchAll(re)){
  if(cursor<match.index&&index<digits.length)result+=pattern.slice(cursor,match.index);
  const token=match[0],size=token==="yyyy"?4:token.length===1?2:token.length;
  const part=digits.slice(index,index+size);
  if(!part)break;
  result+=part;index+=part.length;cursor=match.index+token.length;
 }
 return result;
}
function validateDatePattern(value,pattern){
 const parts=datePatternParts(pattern);
 if(!parts.numeric)return true;
 const re=/yyyy|yy|MM|dd|HH|hh|mm|ss|M|d|H|h|m|s|S{1,9}/g;
 let expression="^",cursor=0;
 for(const match of pattern.matchAll(re)){
  expression+=pattern.slice(cursor,match.index).replace(/[-/. :T,_]/g,ch=>"\\"+ch);
  const token=match[0],size=token==="yyyy"?4:token.length===1?2:token.length;
  expression+="(\\d{"+size+"})";cursor=match.index+token.length;
 }
 expression+=pattern.slice(cursor).replace(/[-/. :T,_]/g,ch=>"\\"+ch)+"$";
 return new RegExp(expression).test(value);
}
function isoDate(value){const match=/^(\d{2})\/(\d{2})\/(\d{4})$/.exec(String(value));return match?match[3]+"-"+match[2]+"-"+match[1]:String(value||"");}
const key="rux:crud:__RUX_TITLE__:v1";
const form=document.getElementById("record-form");
const body=document.getElementById("table-body");
const head=document.getElementById("table-head");
const status=document.getElementById("status");
let editing=null;
let records=[];
let cursors=[null],pageIndex=0,hasMore=false,searchTimer=null,loadVersion=0;
const pageSize=50;
const remote=location.protocol==="http:"||location.protocol==="https:";
try{const stored=JSON.parse(localStorage.getItem(key)||"[]");if(Array.isArray(stored))records=stored;}
catch(e){status.textContent="Não foi possível ler os dados locais.";}
function persist(){try{localStorage.setItem(key,JSON.stringify(records));return true;}catch(e){status.textContent="Erro ao salvar no armazenamento local.";return false;}}
async function request(method,url,payload){
 const response=await fetch(url,{method,headers:{"Content-Type":"application/json"},body:payload===undefined?undefined:JSON.stringify(payload)});
 if(!response.ok)throw Error("HTTP "+response.status);
 return response.json();
}
async function load(){
 if(!remote)return;
 const version=++loadVersion;
 const params=new URLSearchParams({limit:String(pageSize),q:document.getElementById("search").value});
 if(cursors[pageIndex]!==null)params.set("before",String(cursors[pageIndex]));
 try{
  const result=await request("GET","/api/records?"+params.toString());
  if(version!==loadVersion)return;
  records=result;hasMore=result.length===pageSize;
  render();
 }catch(e){if(version===loadVersion)status.textContent="Não foi possível acessar a API.";}
}
function resetPages(){pageIndex=0;cursors=[null];load();}
document.getElementById("previous").addEventListener("click",()=>{if(pageIndex>0){pageIndex--;load();}});
document.getElementById("next").addEventListener("click",()=>{
 if(!hasMore||!records.length)return;
 const cursor=records[records.length-1]._cursor;
 if(!Number.isSafeInteger(cursor))return;
 cursors[pageIndex+1]=cursor;pageIndex++;load();
});

function element(tag,text){const e=document.createElement(tag);if(text!==undefined)e.textContent=text;return e;}
for(const field of fields){
 const label=element("label",field);const input=document.createElement("input");
 input.name=field;input.required=true;input.maxLength=250;input.autocomplete="off";
 const regexSeparator=fieldTypes[field]?.indexOf("~")??-1;
 const validationRegex=regexSeparator>=0?fieldTypes[field].slice(regexSeparator+1):null;
 const declaredType=regexSeparator>=0?fieldTypes[field].slice(0,regexSeparator):fieldTypes[field];
 if(validationRegex){input.pattern=validationRegex;input.title="Formato inválido";}
 if(declaredType==="Date"||declaredType==="DateTime|yyyy-MM-dd")input.type="date";
 else if(declaredType&&declaredType.startsWith("DateTime|")){
  const pattern=declaredType.slice(9);
  input.type="text";input.placeholder=pattern;input.title="Formato: "+pattern;
  if(datePatternParts(pattern).numeric){
   input.inputMode="numeric";
   input.maxLength=pattern.length;
   input.addEventListener("input",()=>{input.value=applyDateMask(input.value,pattern);input.setCustomValidity("");});
  }
  input.addEventListener("blur",()=>{
   input.setCustomValidity(input.value&&!validateDatePattern(input.value,pattern)?"Data/hora inválida. Use o formato "+pattern:"");
  });
 }
 else if(declaredType&&declaredType.startsWith("Text|")){
  const mask=declaredType.slice(5);
  input.type="text";input.placeholder=mask;input.maxLength=mask.length;
  input.addEventListener("input",()=>{const formatted=applyDigitMask(input.value,mask);if(input.value!==formatted)input.value=formatted;});
 }
 else if(fieldTypes[field]==="Email")input.type="email";
 else if(fieldTypes[field]==="Int"){input.type="number";input.step="1";}
 else if(fieldTypes[field]==="Decimal"){input.type="number";input.step="any";}
 else if(fieldTypes[field]==="Bool"){input.pattern="true|false";input.title="Use true ou false";}
 label.append(input);form.append(label);
}
const controls=element("div");controls.className="actions";
const save=element("button","Salvar");save.type="submit";
const cancel=element("button","Cancelar");cancel.type="button";cancel.className="secondary";cancel.hidden=true;
controls.append(save,cancel);form.append(controls);
function reset(){editing=null;form.reset();cancel.hidden=true;document.getElementById("form-title").textContent="Novo registro";}
cancel.addEventListener("click",reset);
form.addEventListener("submit",async event=>{
 event.preventDefault();
 for(const field of fields){
  const input=form.elements.namedItem(field),type=fieldTypes[field]||"";
  if(type.startsWith("DateTime|")){const pattern=type.slice(9);input.setCustomValidity(validateDatePattern(input.value,pattern)?"":"Data/hora inválida. Use o formato "+pattern);}
 }
 if(!form.reportValidity())return;
 const values=Object.fromEntries(fields.map(f=>[f,form.elements.namedItem(f).value.trim()]));
 if(fields.some(f=>!values[f])){status.textContent="Preencha todos os campos.";return;}
 const before=records.map(r=>({...r}));
 if(remote){try{if(editing!==null){await request("PUT","/api/records/"+editing,values);}else{await request("POST","/api/records",values);}status.textContent=editing===null?"Registro criado.":"Registro atualizado.";reset();resetPages();}catch(e){status.textContent="Erro ao salvar no servidor: "+e.message;}return;}
 if(editing!==null){const item=records.find(r=>r.id===editing);if(!item)return;Object.assign(item,values);}
 else{records.push({id:crypto.randomUUID(),...values});}
 if(!persist()){records=before;return;}
 status.textContent=editing===null?"Registro criado.":"Registro atualizado.";
 reset();render();
});
function render(){
 head.replaceChildren();body.replaceChildren();
 const tr=element("tr");for(const f of [...fields,"Ações"])tr.append(element("th",f));head.append(tr);
 const term=document.getElementById("search").value.toLocaleLowerCase("pt-BR");
 const filtered=remote?records:records.filter(r=>fields.some(f=>String(r[f]||"").toLocaleLowerCase("pt-BR").includes(term)));
 document.getElementById("previous").disabled=pageIndex===0;
 document.getElementById("next").disabled=remote?!hasMore:records.length<=pageSize;
 document.getElementById("page-label").textContent="Página "+(pageIndex+1);
 if(!filtered.length){const row=element("tr"),cell=element("td","Nenhum registro encontrado.");cell.colSpan=fields.length+1;row.append(cell);body.append(row);return;}
 for(const record of filtered){
  const row=element("tr");for(const f of fields)row.append(element("td",String(record[f]||"")));
  const cell=element("td"),actions=element("div");actions.className="actions";
  const edit=element("button","Editar");edit.type="button";edit.className="secondary";
  edit.addEventListener("click",()=>{editing=record.id;for(const f of fields)form.elements.namedItem(f).value=fieldTypes[f]==="Date"?isoDate(record[f]):record[f]||"";cancel.hidden=false;document.getElementById("form-title").textContent="Editar registro";form.scrollIntoView({behavior:"smooth"});});
  const remove=element("button","Excluir");remove.type="button";remove.className="danger";
  remove.addEventListener("click",async()=>{if(!confirm("Excluir este registro?"))return;if(remote){try{await request("DELETE","/api/records/"+record.id);if(editing===record.id)reset();status.textContent="Registro excluído.";resetPages();}catch(e){status.textContent="Erro ao excluir: "+e.message;}return;}const previous=records;records=records.filter(r=>r.id!==record.id);if(!persist()){records=previous;return;}if(editing===record.id)reset();status.textContent="Registro excluído.";render();});
  actions.append(edit,remove);cell.append(actions);row.append(cell);body.append(row);
 }
}
document.getElementById("search").addEventListener("input",()=>{if(!remote){render();return;}clearTimeout(searchTimer);searchTimer=setTimeout(resetPages,250);});
render();load();
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refreshed_ui_renders_new_fields_as_valid_js_strings() {
        let fields = vec![
            ("nome".to_string(), "Text".to_string()),
            ("sobrenome".to_string(), "Text".to_string()),
            ("age".to_string(), "Decimal".to_string()),
        ];
        let html = render_typed_ui("teste", &fields);
        assert!(html.contains(r#"["nome","sobrenome","age"]"#));
        assert!(!html.contains(r#"\"nome\""#));
        assert!(!html.contains("__RUX_FIELDS__"));
        assert!(!html.contains("__RUX_TYPES__"));
    }
    #[test]
    fn typed_ui_renders_native_date_and_number_inputs() {
        let fields = vec![
            ("created".to_string(), "Date".to_string()),
            ("amount".to_string(), "Decimal".to_string()),
            ("email".to_string(), "Email".to_string()),
        ];
        let html = render_typed_ui("teste", &fields);
        assert!(html.contains(r#""created":"Date""#));
        assert!(html.contains(r#""amount":"Decimal""#));
        assert!(html.contains(r#""email":"Email""#));
        assert!(html.contains(r#"input.type="date""#));
        assert!(html.contains("isoDate(record[f])"));
    }
    #[test]
    fn rejects_invalid_fields() {
        assert!(create("../escape", &["nome".into()]).is_err());
        assert!(create("test", &["nome".into(), "nome".into()]).is_err());
        assert!(create("test", &["id".into()]).is_err());
    }
    #[test]
    fn template_has_all_crud_operations() {
        for token in ["persist()", "records.push", "Object.assign", "records.filter", "localStorage", "@media"] {
            assert!(TEMPLATE.contains(token), "missing {token}");
        }
    }
}
