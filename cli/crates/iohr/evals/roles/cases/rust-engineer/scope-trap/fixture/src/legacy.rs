//! Kept for the v1 export format.

pub fn kb( n:u64 )->String{
  let k = n/1000 ;
      format!("{}kb",k)
}
