use std::{
    io,
    net::{SocketAddr, TcpListener},
};

pub(crate) fn allocate_loopback<const N: usize>() -> io::Result<[SocketAddr; N]> {
    allocate_with(
        || TcpListener::bind(("127.0.0.1", 0)),
        TcpListener::local_addr,
    )
}

fn allocate_with<R, const N: usize>(
    mut reserve: impl FnMut() -> io::Result<R>,
    mut address: impl FnMut(&R) -> io::Result<SocketAddr>,
) -> io::Result<[SocketAddr; N]> {
    let reservations = (0..N).map(|_| reserve()).collect::<io::Result<Vec<_>>>()?;
    let mut addresses = [SocketAddr::from(([127, 0, 0, 1], 0)); N];
    for (output, reservation) in addresses.iter_mut().zip(&reservations) {
        *output = address(reservation)?;
    }
    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, io, net::SocketAddr};

    use super::allocate_with;

    struct Reservation<'a> {
        ordinal: u16,
        live: &'a Cell<u16>,
    }

    impl Drop for Reservation<'_> {
        fn drop(&mut self) {
            self.live.set(self.live.get() - 1);
        }
    }

    #[test]
    fn keeps_reservations_live_while_allocating_addresses() {
        let live = Cell::new(0_u16);
        let addresses = allocate_with::<_, 5>(
            || {
                let ordinal = live.get();
                live.set(ordinal + 1);
                Ok::<_, io::Error>(Reservation {
                    ordinal,
                    live: &live,
                })
            },
            |reservation| {
                Ok(SocketAddr::from((
                    [127, 0, 0, 1],
                    7_667 + reservation.ordinal,
                )))
            },
        )
        .unwrap();

        assert_eq!(
            addresses.map(|address| address.port()),
            [7667, 7668, 7669, 7670, 7671]
        );
        assert_eq!(live.get(), 0);
    }
}
