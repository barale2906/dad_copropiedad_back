//! Macros internas para reducir código repetitivo en enumeraciones persistidas como `TEXT`.

/// Implementa `as_str`, `TODOS`, `Display` y `TryFrom<&str>` para un enum cuyo valor se
/// persiste como `TEXT` + `CHECK` en PostgreSQL. El texto es el contrato con la base de datos:
/// **no cambiarlo sin una migración**.
macro_rules! enum_texto {
    ($tipo:ident { $($variante:ident => $texto:literal),+ $(,)? }) => {
        impl $tipo {
            /// Todas las variantes, en orden de declaración.
            pub const TODOS: &'static [Self] = &[$(Self::$variante),+];

            /// Representación persistida en la base de datos.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variante => $texto),+
                }
            }
        }

        impl ::std::fmt::Display for $tipo {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::convert::TryFrom<&str> for $tipo {
            type Error = $crate::DominioError;

            fn try_from(valor: &str) -> Result<Self, Self::Error> {
                match valor {
                    $($texto => Ok(Self::$variante),)+
                    otro => Err($crate::DominioError::ValorDesconocido {
                        tipo: stringify!($tipo),
                        valor: otro.to_owned(),
                    }),
                }
            }
        }
    };
}
