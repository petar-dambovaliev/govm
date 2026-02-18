use super::{allocate, Object, Type};
use std::alloc::Layout;
use std::ptr;

#[repr(C)]
pub struct Channel {
    pub sender: async_channel::Sender<Object>,
    pub receiver: async_channel::Receiver<Object>,
    pub capacity: usize,
    pub closed: bool,
    pub ack_sender: Option<async_channel::Sender<()>>,
    pub ack_receiver: Option<async_channel::Receiver<()>>,
}

impl Channel {
    pub fn new(capacity: usize) -> Object {
        let (sender, receiver) = async_channel::bounded(capacity.max(1));

        let (ack_sender, ack_receiver) = if capacity == 0 {
            let (s, r) = async_channel::bounded(1);
            (Some(s), Some(r))
        } else {
            (None, None)
        };

        let ch = Channel {
            sender,
            receiver,
            capacity,
            closed: false,
            ack_sender,
            ack_receiver,
        };

        let ptr = allocate(Layout::new::<Channel>());
        unsafe { ptr::write(ptr as *mut Channel, ch) };
        Object::with_type(ptr, Type::Channel)
    }

    pub fn is_unbuffered(&self) -> bool {
        self.capacity == 0
    }

    pub unsafe fn read<'a>(obj: &Object) -> &'a Channel {
        unsafe { &*(obj.as_ptr() as *const Channel) }
    }

    pub unsafe fn read_mut<'a>(obj: &Object) -> &'a mut Channel {
        unsafe { &mut *(obj.as_ptr() as *mut Channel) }
    }
}
