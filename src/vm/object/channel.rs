use super::{allocate, Object, Type};
use std::alloc::Layout;
use std::ptr;

#[repr(C)]
pub struct Channel {
    pub sender: async_channel::Sender<Object>,
    pub receiver: async_channel::Receiver<Object>,
    pub capacity: usize,
    pub closed: bool,
}

impl Channel {
    pub fn new(capacity: usize) -> Object {
        let (sender, receiver) = if capacity == 0 {
            async_channel::bounded(1)
        } else {
            async_channel::bounded(capacity)
        };

        let ch = Channel {
            sender,
            receiver,
            capacity,
            closed: false,
        };

        let ptr = allocate(Layout::new::<Channel>());
        unsafe { ptr::write(ptr as *mut Channel, ch) };
        Object::with_type(ptr, Type::Channel)
    }

    pub unsafe fn read<'a>(obj: &Object) -> &'a Channel {
        unsafe { &*(obj.as_ptr() as *const Channel) }
    }

    pub unsafe fn read_mut<'a>(obj: &Object) -> &'a mut Channel {
        unsafe { &mut *(obj.as_ptr() as *mut Channel) }
    }
}
