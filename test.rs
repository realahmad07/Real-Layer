use std::io::{Read, Write}; fn test(d: &tun::Device) { let mut b = [0; 10]; let mut d2 = d; let _ = d2.read(&mut b); }  
