use myvec::CVec;

fn main() {
    let mut cv: CVec<i32> = CVec::new();
    cv.push(2);
    assert_eq!(cv.len(), 1);
    // println!("cv length: {:?}", cv.len());
}
