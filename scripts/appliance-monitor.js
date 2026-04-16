// Washer+dryer monitor — KVS "cloud": {u,k,ws,wd,ds,dd}
let W_SW=3,D_SW=2;
let START_W=10,STOP_W=3,CONFIRM_S=10;
let W_DONE_S=300,D_DONE_S=120,POLL=2000;

let w={s:"idle",since:0,rs:0,re:0};
let d={s:"idle",since:0,rs:0,re:0};
let cloud=null;

function ts(){return Shelly.getComponentStatus("sys").unixtime;}

function save(){
  Shelly.call("KVS.Set",{key:"mon",
    value:JSON.stringify({w:w,d:d})});
}

function scene(id){
  if(!cloud||!id) return;
  Shelly.call("HTTP.GET",{
    url:cloud.u+"/scene/manual_run?auth_key="+cloud.k+"&id="+id
  },function(r,e){if(e)print("!scene "+e);});
}

function check(a,sw,done_s,ss,sd){
  Shelly.call("Switch.GetStatus",{id:sw},function(r){
    if(!r) return;
    let pw=r.apower,n=ts();
    let e=r.aenergy?r.aenergy.total:0;
    if(a.s==="idle"){
      if(pw>START_W){a.s="starting";a.since=n;}
    }else if(a.s==="starting"){
      if(pw<=START_W){a.s="idle";a.since=0;}
      else if(n-a.since>=CONFIRM_S){
        a.s="running";a.rs=n;a.re=e;a.since=n;
        save();print("sw"+sw+" started");scene(ss);
      }
    }else if(a.s==="running"){
      if(pw<STOP_W){a.s="finishing";a.since=n;}
    }else if(a.s==="finishing"){
      if(pw>=START_W){a.s="running";a.since=n;}
      else if(n-a.since>=done_s){
        let dur=Math.round((n-a.rs)/60);
        let wh=Math.round((e-a.re)*10)/10;
        print("sw"+sw+" done "+dur+"min "+wh+"Wh");
        scene(sd);
        a.s="idle";a.rs=0;a.re=0;a.since=0;save();
      }
    }
  });
}

function tick(){
  check(w,W_SW,W_DONE_S,cloud&&cloud.ws,cloud&&cloud.wd);
  check(d,D_SW,D_DONE_S,cloud&&cloud.ds,cloud&&cloud.dd);
}

Shelly.call("KVS.Get",{key:"cloud"},function(r){
  if(r){try{cloud=JSON.parse(r.value);}catch(e){print("!cloud");}}
  Shelly.call("KVS.Get",{key:"mon"},function(r){
    if(r){try{
      let m=JSON.parse(r.value);
      if(m.w){w=m.w;w.since=ts();}
      if(m.d){d=m.d;d.since=ts();}
      print("rs w:"+w.s+" d:"+d.s);
    }catch(e){print("!mon");}}
    print("mon sw"+W_SW+" sw"+D_SW);
    Timer.set(POLL,true,tick);
  });
});
