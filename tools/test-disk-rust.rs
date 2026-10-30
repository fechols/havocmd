//! Native protocol model for the actual Rust loader; no hardware or network.
//! rustc --edition=2021 --test tools/test-disk-rust.rs -o target/disk-rust-tests
#![allow(dead_code, static_mut_refs)]
use std::ptr;

mod assets {
    pub static mut MUSICDISK_DIRECTORY_CRC: u32 = 0;
    pub const FORMAT_VERSION: u8 = 2;
    pub const TRACK_COUNT: u8 = 21;
    pub const ASSET_COUNT: u8 = 42;
    pub const SAMPLE_RATE: u32 = 16000;
}
#[path = "../target-c64/src/uci.rs"] mod uci;
#[path = "../target-c64/src/musicdisk.rs"] mod musicdisk;
use musicdisk::*;
use uci::*;

static mut MODEL: *mut Model = ptr::null_mut();
unsafe fn model() -> &'static mut Model { &mut *MODEL }
mod platform {
    use super::*;
    pub unsafe fn ram_ptr(address: u16) -> *mut u8 { model().ram.as_mut_ptr().add(address as usize) }
    pub unsafe fn timer_read() -> u32 {
        let m = model(); let value = m.ticks; m.ticks = m.ticks.wrapping_sub(1); value
    }
    pub unsafe fn read(address: u16) -> u8 { model().read(address) }
    pub unsafe fn write(address: u16, value: u8) { model().write(address, value); }
    pub unsafe fn reu_fetch(source: u32, out: *mut u8, count: u16) {
        let m = model(); assert!(source as usize+count as usize <= m.reu.len());
        ptr::copy_nonoverlapping(m.reu.as_ptr().add(source as usize), out, count as usize);
    }
    pub unsafe fn reu_stash(input: *const u8, destination: u32, count: u16) {
        let m = model(); assert!(destination as usize+count as usize <= m.reu.len());
        ptr::copy_nonoverlapping(input, m.reu.as_mut_ptr().add(destination as usize), count as usize);
    }
}

struct Model {
    ram: Vec<u8>, reu: Vec<u8>, headers: Vec<Vec<u8>>,
    wire: Vec<u8>, answer: Vec<u8>, answer_at: usize, packet_end: usize,
    status_at: usize, status_length: usize, reply_state: u8, reply_mode: u8, dir_index: u8,
    opened: Option<u8>, file_position: u32, ticks: u32,
    short_read: bool, wrong_destination: bool, hold_load: bool, held: bool,
    corrupt_header: bool, present: bool, pins: [u8;2],
    pushes: usize, loads: usize, max_writes: usize, max_reads: usize,
    io_writes: usize, io_reads: usize, opened_paths: Vec<String>,
    current_path: Vec<u8>, volume_folder: String,
}
impl Model {
    fn new() -> Self {
        Self { ram: vec![0;65536], reu: vec![0;0x1000000], headers: vec![vec![0;4096];4],
            wire: vec![], answer: vec![], answer_at:0, packet_end:0, status_at:0, status_length:0,
            reply_state:0, reply_mode:0, dir_index:0, opened:None, file_position:0, ticks:u32::MAX,
            short_read:false, wrong_destination:false, hold_load:false, held:false,
            corrupt_header:false, present:true, pins:[0;2], pushes:0, loads:0,
            max_writes:0, max_reads:0, io_writes:0, io_reads:0, opened_paths:vec![],
            current_path:b"/".to_vec(), volume_folder:"/USB11/havocmd/".to_string() }
    }
    fn reply(&mut self, data: Vec<u8>, mode: u8, ok: bool) {
        self.answer=data; self.answer_at=0; self.packet_end=self.answer.len().min(512);
        self.reply_mode=mode; self.status_at=0; self.status_length=if ok {5} else {0};
        self.reply_state=if self.answer.len()>512 {0x30} else {0x20};
        if self.reply_state==0x30 { self.status_length=0; }
    }
    fn directory_reply(&mut self) {
        self.reply(if self.dir_index==0 {b"\x10SD".to_vec()} else {b"\x10USB11".to_vec()},2,self.dir_index!=0);
        if self.dir_index==0 {self.reply_state=0x30;}
    }
    fn process(&mut self) {
        assert!(self.wire.len()>=2 && self.wire[0]==1); self.pushes+=1;
        match self.wire[1] {
            0x12 => self.reply(self.current_path.clone(),0,true),
            0x17 => self.reply(b"/SD".to_vec(),0,true),
            0x11 => {assert_eq!(self.wire,b"\x01\x11/");self.reply(vec![],0,true);}
            0x13 => self.reply(vec![],0,true),
            0x14 => {self.dir_index=0;self.directory_reply();}
            2 => {
                assert_eq!(self.wire[2],1);
                let path=String::from_utf8(self.wire[3..].to_vec()).unwrap();
                self.opened=(0..4).find(|v|path==format!("{}havocmd-{}.reu",self.volume_folder,v+1));
                self.opened_paths.push(path); self.file_position=0;
                self.reply(vec![],0,self.opened.is_some());
            }
            3 => {self.opened=None;self.reply(vec![],0,true);}
            7 => {assert!(self.opened.is_some());self.reply(0x1000000u32.to_le_bytes().to_vec(),0,true);}
            4 => {
                let v=self.opened.unwrap();
                assert_eq!(self.wire,b"\x01\x04\x00\x10");assert_eq!(self.file_position,0);
                let mut data=self.headers[v as usize].clone();
                if self.corrupt_header && v==2 {data[80]^=1;}
                self.reply(data,1,false);
            }
            6 => {assert!(self.opened.is_some());self.file_position=get32(&self.wire,2);self.reply(vec![],0,true);}
            0x21 => {
                let v=self.opened.unwrap();assert_eq!(self.wire.len(),10);self.loads+=1;
                let destination=get32(&self.wire,2);let length=get32(&self.wire,6);
                assert!(length>0 && length<=16384 && self.file_position>=4096 &&
                    self.file_position+length<=0x1000000);
                assert!(destination>=DISK_SLOT0 && destination+length<=DISK_SLOT1+DISK_SLOT_BYTES);
                if self.pins[0]!=0 {assert!(destination>=DISK_SLOT1);}
                if self.pins[1]!=0 {assert!(destination+length<=DISK_SLOT1);}
                let count=length-if self.short_read {self.short_read=false;1} else {0};
                self.reu[destination as usize..(destination+count) as usize].fill(v+1);
                self.file_position+=count;
                let replied_destination=destination+if self.wrong_destination {self.wrong_destination=false;1} else {0};
                self.reply(format!("${count:6x} BYTES LOADED TO REU ${replied_destination:6x}").into_bytes(),0,true);
                if self.hold_load {self.held=true;self.reply_state=0x10;}
            }
            opcode => panic!("unexpected/non-read-only DOS command {opcode:02x}"),
        }
        self.wire.clear();
    }
    fn read(&mut self,address:u16)->u8 {
        if address==0xdf1d {return if self.present {0xc9} else {0};}
        if address==0xdf1c {
            if self.held {return 0x10;}
            return self.reply_state | if self.answer_at<self.packet_end {0x80} else {0} |
                if self.status_at<self.status_length {0x40} else {0};
        }
        self.io_reads+=1;
        if address==0xdf1e {
            assert!(self.answer_at<self.packet_end);let value=self.answer[self.answer_at];self.answer_at+=1;return value;
        }
        assert_eq!(address,0xdf1f);assert!(self.status_at<self.status_length);
        let value=b"00,OK"[self.status_at];self.status_at+=1;value
    }
    fn write(&mut self,address:u16,value:u8) {
        if address==0xdf1d {self.io_writes+=1;assert!(self.wire.len()<112);self.wire.push(value);return;}
        assert_eq!(address,0xdf1c);
        if value==1 {self.process();return;}
        assert!(value==2 && self.reply_state>=0x20);
        assert_eq!(self.answer_at,self.packet_end);assert_eq!(self.status_at,self.status_length);
        if self.reply_mode==2 && self.dir_index==0 {self.dir_index+=1;self.directory_reply();return;}
        if self.packet_end<self.answer.len() {
            self.packet_end=(self.packet_end+512).min(self.answer.len());
            self.reply_state=if self.packet_end==self.answer.len() {0x20} else {0x30};
            self.status_at=0;self.status_length=0;return;
        }
        self.reply_state=0;self.status_at=0;self.status_length=0;
    }
}
fn get32(p:&[u8],offset:usize)->u32 {u32::from_le_bytes(p[offset..offset+4].try_into().unwrap())}
fn put32(p:&mut[u8],offset:usize,value:u32) {p[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
fn crc32(data:&[u8],header:bool)->u32 {
    let mut crc=!0u32;
    for (i,&value) in data.iter().enumerate() {
        crc^=if header && (32..36).contains(&i) {0} else {value as u32};
        for _ in 0..8 {crc=if crc&1!=0 {(crc>>1)^0xedb88320} else {crc>>1};}
    }
    !crc
}
unsafe fn build_headers(count: u8) {
    let m=model();let h=&mut m.headers[0];
    h[..7].copy_from_slice(b"HVMDVOL");h[8]=2;h[11]=16;h[13]=count;h[14]=42;
    put32(h,16,0x1000000);put32(h,20,0x1000000);
    for i in 0..42 {
        let p=&mut h[64+i*64..64+(i+1)*64];p[0]=(i%21) as u8;p[2]=if i<21 {1} else {2};p[3]=1;
        put32(p,4,0x10000+i as u32*0x10000);put32(p,8,if i<21 {20000} else {65536});
        put32(p,16,if i<21 {16000} else {256});put32(p,20,if i<21 {40000} else {256});
        put32(p,24,if i<21 {2} else {256});put32(p,28,if i<21 {3} else {256});
    }
    h[67]=0;put32(h,68,0x1000000-8192);
    assets::MUSICDISK_DIRECTORY_CRC=crc32(&h[64..],false);put32(h,24,assets::MUSICDISK_DIRECTORY_CRC);
    for v in 0..count as usize {
        if v!=0 {m.headers[v]=m.headers[0].clone();}
        m.headers[v][12]=v as u8;let crc=crc32(&m.headers[v],true);put32(&mut m.headers[v],32,crc);
    }
}
unsafe fn step_checked() {
    model().io_reads=0;model().io_writes=0;disk_step();
    let m=model();m.max_reads=m.max_reads.max(m.io_reads);m.max_writes=m.max_writes.max(m.io_writes);
    assert!(m.io_reads<=32 && m.io_writes<=16);
}
unsafe fn finish() {
    let mut guard=100000;
    while disk_busy!=0 && guard!=0 {step_checked();guard-=1;}
    assert!(guard!=0 && disk_busy==0);
}
unsafe fn pin_slot(slot:u8) {disk_pin(slot);model().pins[slot as usize]+=1;}
unsafe fn release_slot(slot:u8) {disk_release(slot);model().pins[slot as usize]-=1;}
unsafe fn invalid_metadata() {
    let saved=assets::MUSICDISK_DIRECTORY_CRC;
    for case in 0..10 {
        let mut data=model().headers[0].clone();
        match case {
            0=>put32(&mut data,64+8,DISK_SLOT_BYTES),
            1=>put32(&mut data,64+24,1),
            2=>put32(&mut data,64+21*64+24,255),
            3=>data[64+3]=4,
            4=>data[64+42*64]=1,
            5=>put32(&mut data,64+16,24000),
            6=>put32(&mut data,64+4,4095),
            7=>put32(&mut data,64+8,0),
            8=>data[13]=0,
            _=>data[13]=5,
        }
        assets::MUSICDISK_DIRECTORY_CRC=crc32(&data[64..],false);
        put32(&mut data,24,assets::MUSICDISK_DIRECTORY_CRC);
        let crc=crc32(&data,true);put32(&mut data,32,crc);
        model().ram[0x4000..0x5000].copy_from_slice(&data);
        assert!(!musicdisk::test_check_header(0,data[13]),"valid CRC bypassed metadata case{case}");
    }
    assets::MUSICDISK_DIRECTORY_CRC=saved;
}

#[test]
fn rust_production_loader_protocol() { unsafe {
    MODEL=Box::into_raw(Box::new(Model::new()));
    assert_eq!(std::mem::size_of::<DiskBundle>(),30);
    build_headers(4);invalid_metadata();
    assert_eq!(disk_init(),1);
    assert_eq!(&model().reu[64..4096],&model().headers[0][64..]);
    assert!(model().opened_paths.contains(&"/USB11/havocmd/havocmd-4.reu".to_string()));
    assert_eq!(disk_request(21),0);assert_eq!(disk_request(0),1);finish();assert_eq!(disk_error,0);
    let current=disk_ready(0);assert!(!current.is_null());let address=(*current).audio_reu;assert_eq!(address,DISK_SLOT0);
    assert_eq!(model().reu[DISK_SLOT0 as usize],1);assert_eq!(model().reu[DISK_SLOT0 as usize+8191],1);
    assert_eq!(model().reu[DISK_SLOT0 as usize+8192],2);
    let panorama=(*current).panorama_reu;assert_eq!(panorama,DISK_SLOT0+20000);pin_slot((*current).slot);
    let before=model().loads;assert_eq!(disk_request(1),1);assert_eq!(disk_request(2),0);finish();
    assert!(model().loads>before && !disk_ready(1).is_null() && disk_ready(0)==current);
    pin_slot(1);assert_eq!(disk_request(2),0);release_slot(1);
    model().short_read=true;assert_eq!(disk_request(2),1);finish();assert_eq!(disk_error,DISK_SHORT_READ);
    assert!(disk_ready(0)==current && disk_ready(2).is_null());
    model().wrong_destination=true;assert_eq!(disk_request(2),1);finish();assert_eq!(disk_error,DISK_SHORT_READ);
    assert!(disk_ready(0)==current && disk_ready(2).is_null());
    model().hold_load=true;assert_eq!(disk_request(2),1);
    let mut guard=10000;while !model().held && guard!=0 {step_checked();guard-=1;}assert!(guard!=0);
    model().ticks=model().ticks.wrapping_sub(31_000_000);step_checked();
    assert!(disk_error==DISK_TIMEOUT && disk_busy!=0 && disk_request(3)==0 && disk_ready(0)==current);
    model().held=false;model().hold_load=false;model().reply_state=0x20;finish();
    assert!(disk_error==DISK_TIMEOUT && disk_ready(2).is_null());
    assert_eq!(disk_request(2),1);finish();assert!(!disk_ready(2).is_null() && disk_ready(0)==current);
    release_slot(0);
    // Longest accepted path:94 bytes, then the loader appends a slash. The
    // resulting open command is111 bytes and must keep its final filename.
    model().current_path=vec![b'a';94];model().current_path[0]=b'/';
    model().volume_folder=String::from_utf8(model().current_path.clone()).unwrap()+"/";
    model().opened_paths.clear();assert_eq!(disk_init(),1);
    assert_eq!(model().opened_paths.len(),4);
    assert!(model().opened_paths.iter().all(|p|p.len()==108 && p.ends_with(".reu")));
    model().current_path=b"/".to_vec();model().volume_folder="/USB11/havocmd/".to_string();
    model().corrupt_header=true;assert!(disk_init()==0 && disk_error==DISK_HEADER);
    // A two-volume release opens exactly two files and still loads an asset
    // that crosses from volume 1 into volume 2; an entry beyond the count is rejected.
    model().corrupt_header=false;build_headers(2);model().opened_paths.clear();
    assert_eq!(disk_init(),1);
    assert!(model().opened_paths.contains(&"/USB11/havocmd/havocmd-2.reu".to_string()));
    assert!(model().opened_paths.iter().all(|p|!p.ends_with("havocmd-3.reu")&&!p.ends_with("havocmd-4.reu")));
    assert_eq!(disk_request(0),1);finish();assert_eq!(disk_error,0);assert!(!disk_ready(0).is_null());
    {
        let mut data=model().headers[0].clone();data[64+3]=2;
        assets::MUSICDISK_DIRECTORY_CRC=crc32(&data[64..],false);put32(&mut data,24,assets::MUSICDISK_DIRECTORY_CRC);
        let crc=crc32(&data,true);put32(&mut data,32,crc);
        model().ram[0x4000..0x5000].copy_from_slice(&data);
        assert!(!musicdisk::test_check_header(0,2),"entry volume beyond the count was accepted");
    }
    build_headers(4);

    // Exercise the actual parser with caller-owned response storage; no DMA.
    let command=[1,0x12];let mut response=*b"$  4000 BYTES LOADED TO REU $501000";
    assert_eq!(uci_start(command.as_ptr(),1,response.as_mut_ptr(),35,0),0);
    assert_eq!(uci_start(command.as_ptr(),113,response.as_mut_ptr(),35,0),0);
    assert_eq!(uci_start(command.as_ptr(),2,response.as_mut_ptr(),35,0),1);
    uci_response_length=35;
    assert_eq!(uci_transfer_matches(16384,DISK_SLOT1),1);
    assert_eq!(uci_transfer_matches(16383,DISK_SLOT1),0);
    assert_eq!(uci_transfer_matches(16384,DISK_SLOT0),0);
    response[28]=b'!';assert_eq!(uci_transfer_matches(16384,DISK_SLOT1),0);
    response[28]=b'$';response[4]=b' ';assert_eq!(uci_transfer_matches(16384,DISK_SLOT1),0);
    // Independent standard-library parser checks every possible byte in each
    // response position, including both cases and only leading field spaces.
    fn reference(bytes:&[u8],count:u32,destination:u32)->bool {
        if bytes.len()!=35 || bytes[0]!=b'$' || &bytes[7..29]!=b" BYTES LOADED TO REU $" {return false;}
        let number=|field:&[u8]|std::str::from_utf8(field).ok().and_then(|s| {
            let digits=s.trim_start_matches(' ');
            if !digits.bytes().all(|c|c.is_ascii_hexdigit()) {return None;}
            u32::from_str_radix(digits,16).ok()
        });
        number(&bytes[1..7])==Some(count) && number(&bytes[29..35])==Some(destination)
    }
    let templates=[(*b"$  4000 BYTES LOADED TO REU $501000",0x4000,0x501000),
                   (*b"$Abc012 BYTES LOADED TO REU $feABcD",0xabc012,0xfeabcd)];
    for (template,count,destination) in templates {
        for position in 0..35 {
            for byte in 0..=255 {
                response=template;response[position]=byte;
                assert_eq!(uci_transfer_matches(count,destination)!=0,reference(&response,count,destination),
                    "reply position{position} byte{byte}");
            }
        }
    }
    // uci_start was only primed for parser checks; no command was sent.
    uci_busy=0;
    assert_eq!(uci_start(command.as_ptr(),2,response.as_mut_ptr(),0,0),1);
    let mut guard=10000;while uci_busy!=0 && guard!=0 {uci_step();guard-=1;}
    assert!(guard!=0 && uci_error==UCI_OVERFLOW && uci_complete!=0);
    model().present=false;assert!(disk_init()==0 && disk_error==DISK_NO_UCI);
    println!("PASS Rust: USB11/94-byte path discovery, two- and four-volume CRCs, 8 invalid metadata cases, volume crossing, pinned slots, short count/destination, timeout drain/retry,17920 parser mutations and overflow; max{} command/{} reply bytes per step ({} commands)",model().max_writes,model().max_reads,model().pushes);
    drop(Box::from_raw(MODEL));MODEL=ptr::null_mut();
} }
