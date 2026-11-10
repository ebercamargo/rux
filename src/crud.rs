use std::{fs, path::Path};

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn create(name: &str, fields: &[String]) -> Result<(), String> {
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
    let html = TEMPLATE.replace("__RUX_TITLE__", name).replace("__RUX_FIELDS__", &fields_js);
    fs::create_dir(directory).map_err(|e| e.to_string())?;
    fs::write(directory.join("index.html"), html).map_err(|e| e.to_string())?;
    println!("CRUD created: {name}/index.html");
    println!("Open {name}/index.html in any modern desktop or mobile browser.");
    println!("Data is stored locally in that browser; shared server storage and native apps are not implemented yet.");
    Ok(())
}

const TEMPLATE: &str = r##"<!doctype html>
<html lang="pt-BR">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>RUX | __RUX_TITLE__</title>
<style>
:root{font-family:system-ui,-apple-system,sans-serif;color-scheme:light;background:#f5f6fa;color:#202333}
*{box-sizing:border-box}body{margin:0;padding:24px}main{max-width:1050px;margin:auto}
h1{margin:0 0 8px}p{color:#515769}section{background:white;border:1px solid #e2e5ee;border-radius:14px;padding:22px;margin-top:20px}
form{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:14px;align-items:end}
label{display:grid;gap:6px;font-weight:600}input{width:100%;padding:12px;border:1px solid #bfc5d2;border-radius:8px;font:inherit}
button{padding:11px 15px;border:0;border-radius:8px;background:#4d49bd;color:white;font:inherit;cursor:pointer}
button.secondary{background:#596273}button.danger{background:#b5263d}
.actions{display:flex;gap:8px;flex-wrap:wrap}.toolbar{display:flex;justify-content:space-between;align-items:center;gap:12px;flex-wrap:wrap}
#search{max-width:320px}table{width:100%;border-collapse:collapse}th,td{text-align:left;padding:12px;border-bottom:1px solid #e5e7ee}
.table-scroll{overflow-x:auto}#status{min-height:24px}button:focus-visible,input:focus-visible{outline:3px solid #aaa5ff}
@media(max-width:640px){body{padding:12px}section{padding:14px}th,td{padding:9px}}
</style>
</head>
<body><main>
<header><h1>CRUD __RUX_TITLE__</h1><p>RUX • Cadastro multiplataforma no navegador</p></header>
<section><h2 id="form-title">Novo registro</h2><form id="record-form"></form><p id="status" role="status" aria-live="polite"></p></section>
<section><div class="toolbar"><h2>Registros</h2><input id="search" type="search" placeholder="Pesquisar..." aria-label="Pesquisar registros"></div>
<div class="table-scroll"><table><thead id="table-head"></thead><tbody id="table-body"></tbody></table></div></section>
</main>
<script>
"use strict";
const fields=[__RUX_FIELDS__];
const key="rux:crud:__RUX_TITLE__:v1";
const form=document.getElementById("record-form");
const body=document.getElementById("table-body");
const head=document.getElementById("table-head");
const status=document.getElementById("status");
let editing=null;
let records=[];
try{const stored=JSON.parse(localStorage.getItem(key)||"[]");if(Array.isArray(stored))records=stored;}
catch(e){status.textContent="Não foi possível ler os dados locais.";}
function persist(){try{localStorage.setItem(key,JSON.stringify(records));return true;}catch(e){status.textContent="Erro ao salvar no armazenamento local.";return false;}}
function element(tag,text){const e=document.createElement(tag);if(text!==undefined)e.textContent=text;return e;}
for(const field of fields){
 const label=element("label",field);const input=document.createElement("input");
 input.name=field;input.required=true;input.maxLength=250;input.autocomplete="off";
 label.append(input);form.append(label);
}
const controls=element("div");controls.className="actions";
const save=element("button","Salvar");save.type="submit";
const cancel=element("button","Cancelar");cancel.type="button";cancel.className="secondary";cancel.hidden=true;
controls.append(save,cancel);form.append(controls);
function reset(){editing=null;form.reset();cancel.hidden=true;document.getElementById("form-title").textContent="Novo registro";}
cancel.addEventListener("click",reset);
form.addEventListener("submit",event=>{
 event.preventDefault();
 if(!form.reportValidity())return;
 const values=Object.fromEntries(fields.map(f=>[f,form.elements.namedItem(f).value.trim()]));
 if(fields.some(f=>!values[f])){status.textContent="Preencha todos os campos.";return;}
 const before=records.map(r=>({...r}));
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
 const filtered=records.filter(r=>fields.some(f=>String(r[f]||"").toLocaleLowerCase("pt-BR").includes(term)));
 if(!filtered.length){const row=element("tr"),cell=element("td","Nenhum registro encontrado.");cell.colSpan=fields.length+1;row.append(cell);body.append(row);return;}
 for(const record of filtered){
  const row=element("tr");for(const f of fields)row.append(element("td",String(record[f]||"")));
  const cell=element("td"),actions=element("div");actions.className="actions";
  const edit=element("button","Editar");edit.type="button";edit.className="secondary";
  edit.addEventListener("click",()=>{editing=record.id;for(const f of fields)form.elements.namedItem(f).value=record[f]||"";cancel.hidden=false;document.getElementById("form-title").textContent="Editar registro";form.scrollIntoView({behavior:"smooth"});});
  const remove=element("button","Excluir");remove.type="button";remove.className="danger";
  remove.addEventListener("click",()=>{if(!confirm("Excluir este registro?"))return;const previous=records;records=records.filter(r=>r.id!==record.id);if(!persist()){records=previous;return;}if(editing===record.id)reset();status.textContent="Registro excluído.";render();});
  actions.append(edit,remove);cell.append(actions);row.append(cell);body.append(row);
 }
}
document.getElementById("search").addEventListener("input",render);
render();
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::*;
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
